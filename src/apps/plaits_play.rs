//! Plaits' play layer: everything that turns the device's surfaces into
//! one playable instrument, kept apart from the Plaits menu/engine code
//! (plaits.rs) so it can be tested on its own.
//!
//! - **Pad layers** (F2 cycles, L1 held peeks at Controls):
//!   Notes (in key), Chords (a chord per pad, built from the key),
//!   Controls (16 parameters on the pads), Moments (16 saved sounds).
//! - **Voices**: one allocator for every note source -- pads, a MIDI
//!   keyboard (real note numbers + velocity) and the right hand as a
//!   theremin -- so they all play the same 16-voice Plaits engine.
//! - **Expression**: joystick, two depth sensors, velocity, pitch bend,
//!   mod wheel, aftertouch and an audio-input envelope follower, each
//!   routed to a Plaits control (`Target`). Values are offsets on top of
//!   the knob settings, never overwriting them.
//! - **Moments**: 16 whole-sound snapshots, saved to the SD card
//!   (`saves/plaits/moments.json` in the sim).

use crate::app::music_scales::SCALE_TYPES;
use std::path::PathBuf;

pub const LAYER_NAMES: [&str; 4] = ["NOTES", "CHORDS", "CONTROLS", "MOMENTS"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Notes,
    Chords,
    Controls,
    Moments,
}

impl Layer {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn next(self) -> Layer {
        match self {
            Layer::Notes => Layer::Chords,
            Layer::Chords => Layer::Controls,
            Layer::Controls => Layer::Moments,
            Layer::Moments => Layer::Notes,
        }
    }
    pub fn label(self) -> &'static str {
        LAYER_NAMES[self.index()]
    }
}

/// Where an expression source is routed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Off,
    Harmonics,
    Timbre,
    Morph,
    Decay,
    Colour,
    Pitch,
}

pub const TARGETS: [Target; 7] = [Target::Off, Target::Harmonics, Target::Timbre, Target::Morph, Target::Decay, Target::Colour, Target::Pitch];

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Off => "Off",
            Target::Harmonics => "Harmonics",
            Target::Timbre => "Timbre",
            Target::Morph => "Morph",
            Target::Decay => "Decay",
            Target::Colour => "Colour",
            Target::Pitch => "Pitch",
        }
    }
    pub fn cycle(self, step: i32) -> Target {
        let i = TARGETS.iter().position(|t| *t == self).unwrap_or(0) as i32;
        TARGETS[(i + step).rem_euclid(TARGETS.len() as i32) as usize]
    }
    /// Index into `Perf::offsets` (H, T, M, D, Colour, Pitch).
    fn slot(self) -> Option<usize> {
        match self {
            Target::Off => None,
            Target::Harmonics => Some(0),
            Target::Timbre => Some(1),
            Target::Morph => Some(2),
            Target::Decay => Some(3),
            Target::Colour => Some(4),
            Target::Pitch => Some(5),
        }
    }
}

/// How each surface is routed. Defaults are chosen so a first-time
/// player gets something musical from every surface without opening
/// a menu.
#[derive(Clone, Copy, Debug)]
pub struct ExprSettings {
    pub stick_x: Target,
    pub stick_y: Target,
    pub hand_l: Target,
    pub hand_r: Target,
    /// Right hand plays a note by itself when nothing else is held.
    pub theremin: bool,
    /// Semitones the right hand spans (bend or theremin range).
    pub hand_range: i32,
    /// Semitones of MIDI pitch bend.
    pub bend_range: i32,
    /// Harder velocity opens the low-pass gate (brighter), like an accent.
    pub vel_colour: bool,
    pub mod_wheel: Target,
    pub aftertouch: Target,
    pub audio_in: Target,
    /// Chords layer stacks a 7th on top.
    pub sevenths: bool,
}

impl Default for ExprSettings {
    fn default() -> Self {
        Self {
            stick_x: Target::Timbre,
            stick_y: Target::Morph,
            hand_l: Target::Harmonics,
            hand_r: Target::Pitch,
            theremin: true,
            hand_range: 12,
            bend_range: 2,
            vel_colour: true,
            mod_wheel: Target::Morph,
            aftertouch: Target::Timbre,
            audio_in: Target::Off,
            sevenths: false,
        }
    }
}

/// The surfaces' current readings, gathered once per frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct Surfaces {
    pub stick: [f32; 2],
    pub hands: [f32; 2],
    pub pitch_bend: f32,
    pub mod_wheel: f32,
    pub aftertouch: f32,
    /// Audio-input envelope, 0..1.
    pub audio_env: f32,
    /// Velocity (0..1) of the newest sounding note, for the accent.
    pub velocity: f32,
}

/// Offsets the audio thread adds on top of the knob values:
/// [Harmonics, Timbre, Morph, Decay, Colour (0..1 units), Pitch (semitones)].
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Perf {
    pub offsets: [f32; 6],
}

/// Below this a depth sensor reads "no hand" (sensor noise floor).
pub const HAND_FLOOR: f32 = 0.02;

/// Maps the surfaces to Plaits offsets. `hand_playing` = the right hand
/// is itself playing a theremin note, so it must not also bend pitch.
/// `stick_hold` is subtracted from the stick (see `PlayState::keep`).
pub fn perf(s: &ExprSettings, x: &Surfaces, stick_hold: [f32; 2], hand_playing: bool) -> Perf {
    let mut p = Perf::default();
    let mut add = |t: Target, amount: f32, pitch_semis: f32| {
        if let Some(i) = t.slot() {
            p.offsets[i] += if i == 5 { pitch_semis } else { amount };
        }
    };
    let sx = x.stick[0] - stick_hold[0];
    let sy = x.stick[1] - stick_hold[1];
    // Stick: bipolar, half the control's range each way; as pitch, +-2 semitones.
    add(s.stick_x, sx * 0.5, sx * 2.0);
    add(s.stick_y, sy * 0.5, sy * 2.0);
    let hl = if x.hands[0] > HAND_FLOOR { x.hands[0] } else { 0.0 };
    let hr = if x.hands[1] > HAND_FLOOR { x.hands[1] } else { 0.0 };
    add(s.hand_l, hl * 0.6, hl * s.hand_range as f32);
    if !(hand_playing && s.hand_r == Target::Pitch) {
        add(s.hand_r, hr * 0.6, hr * s.hand_range as f32);
    }
    add(s.mod_wheel, x.mod_wheel * 0.5, x.mod_wheel * 2.0);
    add(s.aftertouch, x.aftertouch * 0.5, x.aftertouch * 1.0);
    add(s.audio_in, x.audio_env * 0.6, x.audio_env * 2.0);
    if s.vel_colour {
        p.offsets[4] += (x.velocity - 0.6) * 0.5;
    }
    p.offsets[5] += x.pitch_bend * s.bend_range as f32;
    p
}

// ------------------------------------------------------------ voices

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum KeySrc {
    /// Pad index, chord tone.
    Pad(u8, u8),
    Midi(u8),
    Hand,
}

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub src: Option<KeySrc>,
    pub note: f32,
    pub vel: f32,
    pub gate: bool,
    age: u64,
}

impl Slot {
    const FREE: Slot = Slot { src: None, note: 48.0, vel: 1.0, gate: false, age: 0 };
}

/// 16-voice allocator shared by every note source. Releasing keeps the
/// slot's note (so the envelope's release tail stays on pitch) until it
/// is stolen.
pub struct Voices {
    pub slots: [Slot; 16],
    clock: u64,
}

impl Default for Voices {
    fn default() -> Self {
        Self { slots: [Slot::FREE; 16], clock: 0 }
    }
}

impl Voices {
    /// Bring the held set in line with `wanted` (src, note, velocity).
    pub fn sync(&mut self, wanted: &[(KeySrc, f32, f32)]) {
        for s in self.slots.iter_mut() {
            if s.gate && !wanted.iter().any(|w| Some(w.0) == s.src) {
                s.gate = false;
            }
        }
        for &(src, note, vel) in wanted {
            if let Some(s) = self.slots.iter_mut().find(|s| s.gate && s.src == Some(src)) {
                s.note = note; // continuous sources (the hand) glide
                continue;
            }
            self.clock += 1;
            let idx = self
                .slots
                .iter()
                .position(|s| !s.gate && s.src.is_none())
                .or_else(|| (0..16).filter(|&i| !self.slots[i].gate).min_by_key(|&i| self.slots[i].age))
                .unwrap_or_else(|| (0..16).min_by_key(|&i| self.slots[i].age).unwrap_or(0));
            self.slots[idx] = Slot { src: Some(src), note, vel, gate: true, age: self.clock };
        }
    }

    /// The most recently started sounding voice (mono priority: last note).
    pub fn newest(&self) -> Option<&Slot> {
        self.slots.iter().filter(|s| s.gate).max_by_key(|s| s.age)
    }

    #[allow(dead_code)] // read by the Slint play view and tests
    pub fn sounding(&self) -> usize {
        self.slots.iter().filter(|s| s.gate).count()
    }
}

// ------------------------------------------------------------- harmony

/// The notes of the chord on pitch rank `rank` (0 = lowest pad), in the
/// given scale and root, from `base` (MIDI note of rank 0).
///
/// For 7-note scales this is real diatonic harmony: stack every other
/// scale degree (1-3-5, plus 7 when asked), so each pad is that degree's
/// own triad/7th chord in the key. Other scales (pentatonic, blues,
/// chromatic...) don't stack into triads that way, so the pad's note gets
/// a major or minor triad -- whichever third the scale contains.
pub fn chord_notes(rank: i32, scale: usize, root: i32, base: i32, sevenths: bool) -> Vec<i32> {
    let iv = SCALE_TYPES[scale % SCALE_TYPES.len()].1;
    let len = iv.len() as i32;
    let at = |d: i32| base + root + iv[d.rem_euclid(len) as usize] + 12 * d.div_euclid(len);
    if len == 7 {
        let mut v = vec![at(rank), at(rank + 2), at(rank + 4)];
        if sevenths {
            v.push(at(rank + 6));
        }
        v
    } else {
        let n = at(rank);
        let pc = (n - base - root).rem_euclid(12);
        let has = |semi: i32| iv.contains(&((pc + semi) % 12));
        let third = if has(3) && !has(4) { 3 } else { 4 };
        let mut v = vec![n, n + third, n + 7];
        if sevenths {
            v.push(n + if third == 3 { 10 } else { 11 });
        }
        v
    }
}

// ------------------------------------------------------------- controls

/// The 16 controls of the Controls layer, in pad-rank order (bottom-left
/// first): the sound on the bottom rows, the playing setup on top.
pub const CONTROL_NAMES: [&str; 16] = [
    "Harmonics", "Timbre", "Morph", "Decay", "Attack", "Sustain", "Release", "Colour", "Engine", "Octave", "Root", "Scale",
    "Voices", "Arp", "Arp Rate", "Arp Pattern",
];

/// Knob pages on the play view: each page puts two controls on the knobs.
pub const HERO_PAGES: [[usize; 2]; 4] = [[0, 1], [2, 3], [4, 6], [7, 9]];

// --------------------------------------------------------------- moments

/// One whole sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    pub engine: u32,
    pub harmonics: f32,
    pub timbre: f32,
    pub morph: f32,
    pub decay: f32,
    pub attack: f32,
    pub sustain: f32,
    pub release: f32,
    pub colour: f32,
    pub octave: i32,
}

impl Moment {
    fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "engine": self.engine, "harmonics": self.harmonics, "timbre": self.timbre, "morph": self.morph,
            "decay": self.decay, "attack": self.attack, "sustain": self.sustain, "release": self.release,
            "colour": self.colour, "octave": self.octave,
        })
    }
    fn from_json(v: &serde_json::Value) -> Option<Moment> {
        let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| (x as f32).clamp(0.0, 1.0));
        Some(Moment {
            engine: (v.get("engine")?.as_u64()? as u32).min(23),
            harmonics: f("harmonics")?,
            timbre: f("timbre")?,
            morph: f("morph")?,
            decay: f("decay")?,
            attack: f("attack").unwrap_or(0.0),
            sustain: f("sustain").unwrap_or(1.0),
            release: f("release").unwrap_or(0.3),
            colour: f("colour").unwrap_or(0.5),
            octave: v.get("octave").and_then(|x| x.as_i64()).unwrap_or(0).clamp(-4, 4) as i32,
        })
    }
}

/// 16 moment slots, persisted to the SD card as JSON.
pub struct Moments {
    pub slots: [Option<Moment>; 16],
    path: Option<PathBuf>,
}

impl Moments {
    pub fn default_path() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/plaits/moments.json"))
    }

    /// Loads from `path` (missing or unreadable = empty slots). `None` =
    /// in-memory only (tests, previews).
    pub fn load(path: Option<PathBuf>) -> Moments {
        let mut slots = [None; 16];
        if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&text) {
                for (i, item) in items.iter().take(16).enumerate() {
                    slots[i] = Moment::from_json(item);
                }
            }
        }
        Moments { slots, path }
    }

    pub fn store(&mut self, i: usize, m: Moment) -> Result<(), String> {
        self.slots[i] = Some(m);
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let arr: Vec<serde_json::Value> = self.slots.iter().map(|s| s.map_or(serde_json::Value::Null, |m| m.to_json())).collect();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&arr).unwrap_or_default()).map_err(|e| e.to_string())
    }
}

/// Holding a Moments pad this long stores instead of recalling.
pub const STORE_HOLD_S: f32 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diatonic_chords_follow_the_key() {
        // C major (scale 1): rank 0 = C E G, rank 1 = D F A (minor), rank 4 = G B D.
        assert_eq!(chord_notes(0, 1, 0, 48, false), vec![48, 52, 55]);
        assert_eq!(chord_notes(1, 1, 0, 48, false), vec![50, 53, 57]);
        assert_eq!(chord_notes(4, 1, 0, 48, true), vec![55, 59, 62, 65]);
        // A minor pentatonic (scale 7, root 9): C is in it with Eb not, so C gets major
        let n = chord_notes(1, 7, 9, 48, false);
        assert_eq!(n[0], 48 + 9 + 3);
        assert_eq!(n[1] - n[0], 4);
    }

    #[test]
    fn voices_allocate_release_and_glide() {
        let mut v = Voices::default();
        v.sync(&[(KeySrc::Pad(0, 0), 60.0, 1.0), (KeySrc::Midi(64), 64.0, 0.5)]);
        assert_eq!(v.sounding(), 2);
        assert_eq!(v.newest().unwrap().note, 64.0);
        v.sync(&[(KeySrc::Midi(64), 64.0, 0.5), (KeySrc::Hand, 70.0, 1.0)]);
        assert_eq!(v.sounding(), 2, "pad released, hand added");
        v.sync(&[(KeySrc::Midi(64), 64.0, 0.5), (KeySrc::Hand, 72.5, 1.0)]);
        assert_eq!(v.slots.iter().find(|s| s.src == Some(KeySrc::Hand)).unwrap().note, 72.5, "hand glides in place");
        let many: Vec<_> = (0..20).map(|i| (KeySrc::Midi(i), i as f32, 1.0)).collect();
        v.sync(&many);
        assert_eq!(v.sounding(), 16, "never more than 16 voices");
    }

    #[test]
    fn expression_routes_and_respects_settings() {
        let s = ExprSettings::default();
        let mut x = Surfaces { velocity: 0.6, ..Default::default() };
        assert_eq!(perf(&s, &x, [0.0; 2], false), Perf::default(), "nothing touched = no change");
        x.stick = [1.0, -0.5];
        x.hands = [0.5, 0.25];
        x.pitch_bend = -1.0;
        let p = perf(&s, &x, [0.0; 2], false);
        assert!((p.offsets[1] - 0.5).abs() < 1e-6, "stick x -> timbre");
        assert!((p.offsets[2] + 0.25).abs() < 1e-6, "stick y -> morph");
        assert!((p.offsets[0] - 0.3).abs() < 1e-6, "left hand -> harmonics");
        assert!((p.offsets[5] - (0.25 * 12.0 - 2.0)).abs() < 1e-5, "right hand bends, minus pitch bend");
        let p2 = perf(&s, &x, [0.0; 2], true);
        assert!((p2.offsets[5] + 2.0).abs() < 1e-5, "a theremin hand doesn't also bend");
        let held = perf(&s, &x, [1.0, -0.5], false);
        assert!(held.offsets[1].abs() < 1e-6, "kept stick position cancels");
    }

    #[test]
    fn moments_round_trip_through_the_sd_card() {
        let path = std::env::temp_dir().join(format!("plaits_moments_{}.json", std::process::id()));
        let m = Moment { engine: 13, harmonics: 0.1, timbre: 0.2, morph: 0.3, decay: 0.4, attack: 0.0, sustain: 1.0, release: 0.3, colour: 0.7, octave: -1 };
        let mut a = Moments::load(Some(path.clone()));
        a.store(5, m).unwrap();
        let b = Moments::load(Some(path.clone()));
        assert_eq!(b.slots[5], Some(m));
        assert!(b.slots[0].is_none());
        std::fs::remove_file(path).ok();
    }
}
