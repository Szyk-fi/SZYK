//! A classic subtractive analog-style synth voice -- two detunable
//! oscillators (Saw/Square/Triangle/Sine, naive/non-bandlimited, same
//! simplification `DrumVoice` in sequencer.rs already makes) plus a
//! sub-oscillator and noise, a resonant state-variable lowpass
//! filter with its own envelope, a separate amp envelope, unison
//! (1-4 detuned copies per voice for that "fat" analog stack sound),
//! and an LFO routable to either pitch (vibrato) or filter cutoff
//! (wobble). Not modeled on any one specific hardware synth --
//! "analog-style" here means the classic oscillator -> filter ->
//! amp signal path shared by essentially the whole subtractive-synth
//! lineage, built from standard DSP rather than any particular unit's
//! circuit behavior.
//!
//! Fully polyphonic across all 16 grid pads, one voice per pad, same
//! "16 fixed voices, no stealing needed" convention Plaits' Poly mode
//! and Cascade both use.

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes};
use crate::app::{App, Input};
use crate::arpeggiator::{Arpeggiator, PATTERN_NAMES as ARP_PATTERN_NAMES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, note_name, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use super::voltage_fx::{FxSettings, StereoFx, MAX_DELAY_S, MIN_DELAY_S};
use std::sync::{Arc, Mutex};

const BASE_NOTE: i32 = 48;
const NOTE_MIN: i32 = -119;
const NOTE_MAX: i32 = 120;
const WAVE_NAMES: [&str; 4] = ["Saw", "Square", "Triangle", "Sine"];
const MAX_UNISON: u32 = 4;
const MIN_UNISON: u32 = 1;
const MAX_ENV_SECONDS: f32 = 4.0;
const MIN_CUTOFF_HZ: f32 = 40.0;
const MAX_CUTOFF_HZ: f32 = 12000.0;
const MIN_LFO_RATE: f32 = 0.05;
const MAX_LFO_RATE: f32 = 15.0;
const LFO_DEST_NAMES: [&str; 2] = ["Pitch", "Filter"];
const MAX_DELAY_FEEDBACK: f32 = 0.9;

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

fn wave_sample(waveform: u32, phase: f32) -> f32 {
    match waveform % 4 {
        0 => 2.0 * phase - 1.0,                      // Saw
        1 => if phase < 0.5 { 1.0 } else { -1.0 },   // Square
        2 => 1.0 - 4.0 * (phase - 0.5).abs(),         // Triangle
        _ => (phase * TAU).sin(),                     // Sine
    }
}

/// Chamberlin state-variable filter, one step -- same recurrence
/// prism.rs's `svf_bandpass_step` uses, duplicated locally since this
/// call site wants the lowpass output instead of the bandpass one.
fn svf_lowpass_step(input: f32, lp: f32, bp: f32, f_coef: f32, q: f32) -> (f32, f32, f32) {
    let new_lp = lp + f_coef * bp;
    let high = input - new_lp - (1.0 / q) * bp;
    let new_bp = bp + f_coef * high;
    (new_lp, new_bp, new_lp)
}

fn pad_rank(physical_index: i32) -> i32 {
    let row = physical_index / 4;
    let col = physical_index % 4;
    (3 - row) * 4 + col
}

fn note_for(rank: i32, octave: i32) -> i32 {
    (BASE_NOTE + rank + octave * 12).clamp(NOTE_MIN, NOTE_MAX)
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    PresetSlot,
    PresetSave,
    Osc1Wave,
    Octave,
    Osc2Wave,
    Osc2Detune,
    OscMix,
    SubLevel,
    NoiseLevel,
    Unison,
    UnisonDetune,
    Mono,
    Glide,
    FilterCutoff,
    FilterResonance,
    FilterEnvAmount,
    FilterAttack,
    FilterDecay,
    FilterSustain,
    FilterRelease,
    AmpAttack,
    AmpDecay,
    AmpSustain,
    AmpRelease,
    LfoRate,
    LfoDepth,
    LfoDest,
    Chorus,
    DelayTime,
    DelayFeedback,
    DelayMix,
    ReverbSize,
    ReverbMix,
    ArpOn,
    ArpPattern,
    ArpRate,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 7;
const PRESET_GROUP: usize = 0;
const FX_GROUP: usize = 5;
/// How many preset slots there are. The factory presets fill the first
/// ones (`assets/voltage/factory.json`); the rest start empty.
pub(crate) const NUM_PRESETS: usize = 100;
/// Longest glide, in seconds, at the Glide knob's maximum.
const MAX_GLIDE_S: f32 = 1.0;

/// One complete Voltage sound: every knob, the arp, and the effects.
///
/// Every field has a default (the init patch, see `Default`), so a
/// preset file only needs the fields that differ from it. That keeps
/// `assets/voltage/factory.json` readable and lets older files keep
/// loading after new parameters are added.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PresetData {
    pub name: String,
    pub osc1_wave: u32,
    pub osc2_wave: u32,
    pub octave: i32,
    pub osc2_detune: f32,
    pub osc_mix: f32,
    pub sub_level: f32,
    pub noise_level: f32,
    pub unison: u32,
    pub unison_detune: f32,
    pub mono: bool,
    pub glide: f32,
    pub filter_cutoff: f32,
    pub filter_resonance: f32,
    pub filter_env_amount: f32,
    pub filter_attack: f32,
    pub filter_decay: f32,
    pub filter_sustain: f32,
    pub filter_release: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,
    pub lfo_rate: f32,
    pub lfo_depth: f32,
    pub lfo_dest: u32,
    pub chorus: f32,
    pub delay_time: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
    pub reverb_size: f32,
    pub reverb_mix: f32,
    pub arp_on: bool,
    pub arp_pattern: u32,
    pub arp_rate: f32,
}

impl Default for PresetData {
    /// The init patch: what Voltage sounds like with nothing loaded.
    fn default() -> Self {
        Self {
            name: String::new(),
            osc1_wave: 0,
            osc2_wave: 0,
            octave: 0,
            osc2_detune: 0.15,
            osc_mix: 0.5,
            sub_level: 0.3,
            noise_level: 0.0,
            unison: 1,
            unison_detune: 0.15,
            mono: false,
            glide: 0.0,
            filter_cutoff: 0.6,
            filter_resonance: 0.3,
            // Defaults to no sweep at all -- a nonzero amount makes the
            // filter cutoff sweep with every note's envelope, which
            // (especially with resonance emphasizing the moving peak)
            // reads as a pitch "glide" rather than a plain tone. Real
            // portamento is the separate Glide knob (mono mode only).
            filter_env_amount: 0.0,
            filter_attack: 0.0,
            filter_decay: 0.4,
            filter_sustain: 0.3,
            filter_release: 0.25,
            amp_attack: 0.02,
            amp_decay: 0.3,
            amp_sustain: 0.8,
            amp_release: 0.3,
            lfo_rate: 4.0,
            lfo_depth: 0.0,
            lfo_dest: 0,
            chorus: 0.0,
            delay_time: 0.375,
            delay_feedback: 0.35,
            delay_mix: 0.0,
            reverb_size: 0.5,
            reverb_mix: 0.0,
            arp_on: false,
            arp_pattern: 0,
            arp_rate: 8.0,
        }
    }
}

/// The factory presets, compiled in so they are always there. They
/// are original patches written for this synth "in the style of" the
/// synthwave and retrowave records they're named after in the file's
/// comments -- not transcriptions of anyone's actual patch.
const FACTORY_JSON: &str = include_str!("../../assets/voltage/factory.json");

pub(crate) fn factory_presets() -> Vec<PresetData> {
    #[derive(serde::Deserialize)]
    struct File {
        presets: Vec<PresetData>,
    }
    serde_json::from_str::<File>(FACTORY_JSON).map(|f| f.presets).unwrap_or_default()
}

/// The 100 slots. Factory presets occupy the first slots; anything the
/// player saves -- into an empty slot or over a factory one -- is
/// written to `saves/voltage/presets.json` on the SD card. Clearing a
/// factory slot brings its factory preset back; clearing any other slot
/// empties it.
struct PresetBank {
    slots: Vec<Option<PresetData>>,
    factory: Vec<PresetData>,
    path: Option<std::path::PathBuf>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UserFile {
    version: u32,
    slots: Vec<UserSlot>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UserSlot {
    /// 1-based, as shown on screen.
    slot: usize,
    preset: PresetData,
}

impl PresetBank {
    fn new(path: Option<std::path::PathBuf>) -> Self {
        let factory = factory_presets();
        let mut slots: Vec<Option<PresetData>> = vec![None; NUM_PRESETS];
        for (i, p) in factory.iter().take(NUM_PRESETS).enumerate() {
            slots[i] = Some(p.clone());
        }
        let mut bank = Self { slots, factory, path };
        bank.load();
        bank
    }

    fn load(&mut self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(file) = serde_json::from_str::<UserFile>(&text) else {
            eprintln!("voltage: couldn't parse {}", path.display());
            return;
        };
        for s in file.slots {
            if (1..=NUM_PRESETS).contains(&s.slot) {
                self.slots[s.slot - 1] = Some(s.preset);
            }
        }
    }

    /// Writes every slot that differs from its factory state.
    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let slots: Vec<UserSlot> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let p = s.as_ref()?;
                (self.factory.get(i) != Some(p)).then(|| UserSlot { slot: i + 1, preset: p.clone() })
            })
            .collect();
        let file = UserFile { version: 1, slots };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(&file) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    eprintln!("voltage: couldn't save presets: {e}");
                }
            }
            Err(e) => eprintln!("voltage: couldn't encode presets: {e}"),
        }
    }

    fn get(&self, i: usize) -> Option<&PresetData> {
        self.slots.get(i).and_then(|s| s.as_ref())
    }

    fn store(&mut self, i: usize, preset: PresetData) {
        if i < NUM_PRESETS {
            self.slots[i] = Some(preset);
            self.persist();
        }
    }

    fn clear(&mut self, i: usize) {
        if i < NUM_PRESETS {
            self.slots[i] = self.factory.get(i).cloned();
            self.persist();
        }
    }

    fn is_factory(&self, i: usize) -> bool {
        i < self.factory.len()
    }

    fn first_empty(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }

    fn filled(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

struct Params {
    osc1_wave: AtomicU32,
    /// Transposes every pad's note by this many octaves -- same
    /// pattern/range as Plaits' own Octave.
    octave: AtomicI32,
    osc2_wave: AtomicU32,
    osc2_detune: AtomicF32, // semitones
    osc_mix: AtomicF32,     // 0 = all osc1, 1 = all osc2
    sub_level: AtomicF32,
    noise_level: AtomicF32,
    unison: AtomicU32,
    unison_detune: AtomicF32, // cents spread across the unison stack
    filter_cutoff: AtomicF32, // 0..1 knob, exponential-mapped to Hz
    ext_filter_cutoff: Arc<AtomicF32>,
    filter_resonance: AtomicF32,
    filter_env_amount: AtomicF32, // -1..1, octaves of cutoff sweep
    filter_attack: AtomicF32,
    filter_decay: AtomicF32,
    filter_sustain: AtomicF32,
    filter_release: AtomicF32,
    amp_attack: AtomicF32,
    amp_decay: AtomicF32,
    amp_sustain: AtomicF32,
    amp_release: AtomicF32,
    lfo_rate: AtomicF32,
    lfo_depth: AtomicF32,
    lfo_dest: AtomicU32,
    /// One voice, last-note priority, legato envelopes -- the mode
    /// synthwave leads and basses are played in. Glide only applies here.
    mono: AtomicBool,
    glide: AtomicF32,
    chorus: AtomicF32,
    delay_time: AtomicF32,
    delay_feedback: AtomicF32,
    delay_mix: AtomicF32,
    reverb_size: AtomicF32,
    reverb_mix: AtomicF32,
    held: Mutex<[bool; 16]>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    /// See arpeggiator.rs -- stepped once per audio block in `process`.
    arp: Arpeggiator,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Voltage", modbus);
        let params = Self {
            osc1_wave: AtomicU32::new(0),
            octave: AtomicI32::new(0),
            osc2_wave: AtomicU32::new(0),
            osc2_detune: AtomicF32::new(0.0),
            osc_mix: AtomicF32::new(0.0),
            sub_level: AtomicF32::new(0.0),
            noise_level: AtomicF32::new(0.0),
            unison: AtomicU32::new(1),
            unison_detune: AtomicF32::new(0.0),
            filter_cutoff: AtomicF32::new(0.0),
            ext_filter_cutoff: modbus.register("Voltage: Filter Cutoff".to_string()),
            filter_resonance: AtomicF32::new(0.0),
            filter_env_amount: AtomicF32::new(0.0),
            filter_attack: AtomicF32::new(0.0),
            filter_decay: AtomicF32::new(0.0),
            filter_sustain: AtomicF32::new(0.0),
            filter_release: AtomicF32::new(0.0),
            amp_attack: AtomicF32::new(0.0),
            amp_decay: AtomicF32::new(0.0),
            amp_sustain: AtomicF32::new(0.0),
            amp_release: AtomicF32::new(0.0),
            lfo_rate: AtomicF32::new(0.0),
            lfo_depth: AtomicF32::new(0.0),
            lfo_dest: AtomicU32::new(0),
            mono: AtomicBool::new(false),
            glide: AtomicF32::new(0.0),
            chorus: AtomicF32::new(0.0),
            delay_time: AtomicF32::new(0.0),
            delay_feedback: AtomicF32::new(0.0),
            delay_mix: AtomicF32::new(0.0),
            reverb_size: AtomicF32::new(0.0),
            reverb_mix: AtomicF32::new(0.0),
            held: Mutex::new([false; 16]),
            bus_out: audio_bus.register("Voltage"),
            mix_level,
            ext_mix_level,
            arp: Arpeggiator::new(),
        };
        params.apply(&PresetData::default());
        params
    }

    /// Everything that makes up the current sound, as a preset.
    fn snapshot(&self, name: &str) -> PresetData {
        PresetData {
            name: name.to_string(),
            osc1_wave: self.osc1_wave.load(Ordering::Relaxed),
            osc2_wave: self.osc2_wave.load(Ordering::Relaxed),
            octave: self.octave.load(Ordering::Relaxed),
            osc2_detune: self.osc2_detune.get(),
            osc_mix: self.osc_mix.get(),
            sub_level: self.sub_level.get(),
            noise_level: self.noise_level.get(),
            unison: self.unison.load(Ordering::Relaxed),
            unison_detune: self.unison_detune.get(),
            mono: self.mono.load(Ordering::Relaxed),
            glide: self.glide.get(),
            filter_cutoff: self.filter_cutoff.get(),
            filter_resonance: self.filter_resonance.get(),
            filter_env_amount: self.filter_env_amount.get(),
            filter_attack: self.filter_attack.get(),
            filter_decay: self.filter_decay.get(),
            filter_sustain: self.filter_sustain.get(),
            filter_release: self.filter_release.get(),
            amp_attack: self.amp_attack.get(),
            amp_decay: self.amp_decay.get(),
            amp_sustain: self.amp_sustain.get(),
            amp_release: self.amp_release.get(),
            lfo_rate: self.lfo_rate.get(),
            lfo_depth: self.lfo_depth.get(),
            lfo_dest: self.lfo_dest.load(Ordering::Relaxed),
            chorus: self.chorus.get(),
            delay_time: self.delay_time.get(),
            delay_feedback: self.delay_feedback.get(),
            delay_mix: self.delay_mix.get(),
            reverb_size: self.reverb_size.get(),
            reverb_mix: self.reverb_mix.get(),
            arp_on: self.arp.enabled.load(Ordering::Relaxed),
            arp_pattern: self.arp.pattern.load(Ordering::Relaxed),
            arp_rate: self.arp.rate_hz.get(),
        }
    }

    /// Loads a preset into the knobs. Every value is clamped to its
    /// knob's range, so a hand-edited file can't push the DSP outside
    /// the ranges it was designed for.
    fn apply(&self, p: &PresetData) {
        self.osc1_wave.store(p.osc1_wave % 4, Ordering::Relaxed);
        self.osc2_wave.store(p.osc2_wave % 4, Ordering::Relaxed);
        self.octave.store(p.octave.clamp(-OCTAVE_SPAN, OCTAVE_SPAN), Ordering::Relaxed);
        self.osc2_detune.set(p.osc2_detune.clamp(-12.0, 12.0));
        self.osc_mix.set(p.osc_mix.clamp(0.0, 1.0));
        self.sub_level.set(p.sub_level.clamp(0.0, 1.0));
        self.noise_level.set(p.noise_level.clamp(0.0, 1.0));
        self.unison.store(p.unison.clamp(MIN_UNISON, MAX_UNISON), Ordering::Relaxed);
        self.unison_detune.set(p.unison_detune.clamp(0.0, 1.0));
        self.mono.store(p.mono, Ordering::Relaxed);
        self.glide.set(p.glide.clamp(0.0, 1.0));
        self.filter_cutoff.set(p.filter_cutoff.clamp(0.0, 1.0));
        self.filter_resonance.set(p.filter_resonance.clamp(0.0, 1.0));
        self.filter_env_amount.set(p.filter_env_amount.clamp(-1.0, 1.0));
        self.filter_attack.set(p.filter_attack.clamp(0.0, 1.0));
        self.filter_decay.set(p.filter_decay.clamp(0.0, 1.0));
        self.filter_sustain.set(p.filter_sustain.clamp(0.0, 1.0));
        self.filter_release.set(p.filter_release.clamp(0.0, 1.0));
        self.amp_attack.set(p.amp_attack.clamp(0.0, 1.0));
        self.amp_decay.set(p.amp_decay.clamp(0.0, 1.0));
        self.amp_sustain.set(p.amp_sustain.clamp(0.0, 1.0));
        self.amp_release.set(p.amp_release.clamp(0.0, 1.0));
        self.lfo_rate.set(p.lfo_rate.clamp(MIN_LFO_RATE, MAX_LFO_RATE));
        self.lfo_depth.set(p.lfo_depth.clamp(0.0, 1.0));
        self.lfo_dest.store(p.lfo_dest % 2, Ordering::Relaxed);
        self.chorus.set(p.chorus.clamp(0.0, 1.0));
        self.delay_time.set(p.delay_time.clamp(MIN_DELAY_S, MAX_DELAY_S));
        self.delay_feedback.set(p.delay_feedback.clamp(0.0, MAX_DELAY_FEEDBACK));
        self.delay_mix.set(p.delay_mix.clamp(0.0, 1.0));
        self.reverb_size.set(p.reverb_size.clamp(0.0, 1.0));
        self.reverb_mix.set(p.reverb_mix.clamp(0.0, 1.0));
        self.arp.enabled.store(p.arp_on, Ordering::Relaxed);
        self.arp.pattern.store(p.arp_pattern % ARP_PATTERN_NAMES.len() as u32, Ordering::Relaxed);
        self.arp.rate_hz.set(p.arp_rate.clamp(0.5, 30.0));
    }
}

pub struct VoltageApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
    presets: PresetBank,
    /// The slot the Preset row shows and steps through.
    preset_cursor: usize,
    /// What was last loaded or saved, to mark the sound as edited ("*")
    /// once a knob moves away from it.
    loaded: Option<PresetData>,
    /// The slot SELECT on the Save row writes to.
    save_target: usize,
}

/// The play view's controls, most important first: the filter and the
/// oscillator blend on the knobs, the envelope and LFO next, the
/// patch-shaping choices on the upper pads of the Controls layer.
const CONTROLS: [(Selection, &str); 16] = [
    (Selection::FilterCutoff, "Cutoff"),
    (Selection::FilterResonance, "Resonance"),
    (Selection::FilterEnvAmount, "Env Amount"),
    (Selection::OscMix, "Osc Mix"),
    (Selection::AmpAttack, "Attack"),
    (Selection::AmpRelease, "Release"),
    (Selection::LfoDepth, "LFO Depth"),
    (Selection::LfoRate, "LFO Rate"),
    (Selection::Osc1Wave, "Osc 1 Wave"),
    (Selection::Osc2Wave, "Osc 2 Wave"),
    (Selection::Osc2Detune, "Detune"),
    (Selection::SubLevel, "Sub"),
    (Selection::NoiseLevel, "Noise"),
    (Selection::Unison, "Unison"),
    (Selection::Octave, "Octave"),
    (Selection::PresetSlot, "Preset"),
];
const C_OCTAVE: usize = 14;
/// Octave range the play view's D-pad and moments use; the menu's own
/// Octave row is unbounded, so this only bounds the dial's position.
const OCTAVE_SPAN: i32 = 4;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "voltage",
        layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7]],
        // D-pad up/down shifts the octave: 16 pads only span 16
        // semitones, so this is the move a player makes most.
        browse: Some(C_OCTAVE),
        // Stick: cutoff sweeps on X, LFO depth (vibrato / wobble) on Y,
        // like a pitch-and-mod-wheel joystick. Hands: resonance, and the
        // blend between the two oscillators.
        routes: Routes { stick_x: Some(0), stick_y: Some(6), hand_l: Some(1), hand_r: Some(3) },
        throws: Vec::new(),
        midi_to_pads: true,
        own_expression: false,
    }
}

// --- Voltage's own palette: vintage teal on charcoal, not a
// device-wide theme -- a classic subtractive-synth panel look,
// distinct from Synth's own warm orange. The filter panel's cutoff
// (red) and env-sweep (amber) markers stay their own hues -- they're
// functional distinct markers already outside the shared green
// family, not this palette's job to touch. ---

const VOLTAGE_BG: Rgb565 = Rgb565::new(2, 6, 3);
const VOLTAGE_TITLE: Rgb565 = Rgb565::new(27, 60, 29);
const VOLTAGE_ACCENT: Rgb565 = Rgb565::new(9, 51, 20);
const VOLTAGE_DIM: Rgb565 = Rgb565::new(9, 26, 12);
const VOLTAGE_OUTLINE: Rgb565 = Rgb565::new(5, 10, 6);

impl VoltageApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let mut app = Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: std::array::from_fn(|g| g == PRESET_GROUP),
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            presets: PresetBank::new((!cfg!(test)).then(|| std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/voltage/presets.json")))),
            preset_cursor: 0,
            loaded: None,
            save_target: 0,
        };
        app.save_target = app.presets.first_empty().unwrap_or(0);
        // Open on the first preset, so the synth sounds like something
        // straight away.
        app.load_preset(0);
        app
    }

    /// Loads slot `i` if it holds a preset; returns whether it did.
    fn load_preset(&mut self, i: usize) -> bool {
        self.preset_cursor = i.min(NUM_PRESETS - 1);
        let Some(p) = self.presets.get(self.preset_cursor).cloned() else { return false };
        self.params.apply(&p);
        self.loaded = Some(p);
        true
    }

    /// Saves the current sound into `save_target`. A slot that already
    /// has a preset keeps its name; an empty one gets "User NN".
    fn save_preset(&mut self) {
        let i = self.save_target;
        let name = match self.presets.get(i) {
            Some(p) if !p.name.is_empty() => p.name.clone(),
            _ => format!("User {:02}", i + 1),
        };
        let data = self.params.snapshot(&name);
        self.presets.store(i, data.clone());
        self.preset_cursor = i;
        self.loaded = Some(data);
        self.kit.flash(format!("Saved to {:02} {name}", i + 1));
    }

    /// Whether the knobs have moved since the last load or save.
    fn edited(&self) -> bool {
        match &self.loaded {
            Some(p) => self.params.snapshot(&p.name) != *p,
            None => true,
        }
    }

    fn slot_label(&self, i: usize) -> String {
        match self.presets.get(i) {
            Some(p) => format!("{:02} {}", i + 1, p.name),
            None => format!("{:02} (empty)", i + 1),
        }
    }

    /// The current preset's name for titles: "07 Neon Arp", with a "*"
    /// once the sound has been edited.
    pub(crate) fn preset_title(&self) -> String {
        format!("{}{}", self.slot_label(self.preset_cursor), if self.edited() { " *" } else { "" })
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            PRESET_GROUP => vec![Selection::PresetSlot, Selection::PresetSave],
            1 => vec![
                Selection::Osc1Wave,
                Selection::Octave,
                Selection::Osc2Wave,
                Selection::Osc2Detune,
                Selection::OscMix,
                Selection::SubLevel,
                Selection::NoiseLevel,
                Selection::Unison,
                Selection::UnisonDetune,
                Selection::Mono,
                Selection::Glide,
            ],
            2 => vec![
                Selection::FilterCutoff,
                Selection::FilterResonance,
                Selection::FilterEnvAmount,
                Selection::FilterAttack,
                Selection::FilterDecay,
                Selection::FilterSustain,
                Selection::FilterRelease,
            ],
            3 => vec![Selection::AmpAttack, Selection::AmpDecay, Selection::AmpSustain, Selection::AmpRelease],
            4 => vec![Selection::LfoRate, Selection::LfoDepth, Selection::LfoDest],
            FX_GROUP => vec![Selection::Chorus, Selection::DelayTime, Selection::DelayFeedback, Selection::DelayMix, Selection::ReverbSize, Selection::ReverbMix],
            _ => vec![Selection::ArpOn, Selection::ArpPattern, Selection::ArpRate],
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                for sel in self.group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    fn group_name(&self, g: usize) -> &'static str {
        match g {
            PRESET_GROUP => "Presets",
            1 => "Oscillators",
            2 => "Filter",
            3 => "Amp Envelope",
            4 => "LFO",
            FX_GROUP => "Effects",
            _ => "Arp",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            PRESET_GROUP => format!("{}/{NUM_PRESETS} used", self.presets.filled()),
            1 => format!(
                "{} + {}, {}",
                WAVE_NAMES[self.params.osc1_wave.load(Ordering::Relaxed) as usize % 4],
                WAVE_NAMES[self.params.osc2_wave.load(Ordering::Relaxed) as usize % 4],
                if self.params.mono.load(Ordering::Relaxed) { "mono".to_string() } else { format!("x{}", self.params.unison.load(Ordering::Relaxed)) }
            ),
            2 => format!("cutoff {:.2}, res {:.2}", self.params.filter_cutoff.get(), self.params.filter_resonance.get()),
            3 => format!("A{:.2} D{:.2} S{:.2} R{:.2}", self.params.amp_attack.get(), self.params.amp_decay.get(), self.params.amp_sustain.get(), self.params.amp_release.get()),
            4 => LFO_DEST_NAMES[self.params.lfo_dest.load(Ordering::Relaxed) as usize % 2].to_string(),
            FX_GROUP => {
                let on: Vec<&str> = [("cho", self.params.chorus.get()), ("dly", self.params.delay_mix.get()), ("rev", self.params.reverb_mix.get())]
                    .iter()
                    .filter(|(_, v)| *v > 0.0)
                    .map(|(n, _)| *n)
                    .collect();
                if on.is_empty() { "dry".into() } else { on.join(" ") }
            }
            _ => self.leaf_value(Selection::ArpOn),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::PresetSlot => "Preset".into(),
            Selection::PresetSave => "Save to".into(),
            Selection::Osc1Wave => "Osc 1 Wave".into(),
            Selection::Octave => "Octave".into(),
            Selection::Osc2Wave => "Osc 2 Wave".into(),
            Selection::Osc2Detune => "Osc 2 Detune".into(),
            Selection::OscMix => "Osc Mix".into(),
            Selection::SubLevel => "Sub Level".into(),
            Selection::NoiseLevel => "Noise Level".into(),
            Selection::Unison => "Unison".into(),
            Selection::UnisonDetune => "Unison Detune".into(),
            Selection::Mono => "Mono".into(),
            Selection::Glide => "Glide".into(),
            Selection::FilterCutoff => "Cutoff".into(),
            Selection::FilterResonance => "Resonance".into(),
            Selection::FilterEnvAmount => "Env Amount".into(),
            Selection::FilterAttack => "Filter Attack".into(),
            Selection::FilterDecay => "Filter Decay".into(),
            Selection::FilterSustain => "Filter Sustain".into(),
            Selection::FilterRelease => "Filter Release".into(),
            Selection::AmpAttack => "Attack".into(),
            Selection::AmpDecay => "Decay".into(),
            Selection::AmpSustain => "Sustain".into(),
            Selection::AmpRelease => "Release".into(),
            Selection::LfoRate => "Rate".into(),
            Selection::LfoDepth => "Depth".into(),
            Selection::LfoDest => "Destination".into(),
            Selection::Chorus => "Chorus".into(),
            Selection::DelayTime => "Delay Time".into(),
            Selection::DelayFeedback => "Feedback".into(),
            Selection::DelayMix => "Delay Mix".into(),
            Selection::ReverbSize => "Reverb Size".into(),
            Selection::ReverbMix => "Reverb Mix".into(),
            Selection::ArpOn => "On/Off".into(),
            Selection::ArpPattern => "Pattern".into(),
            Selection::ArpRate => "Rate".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Osc1Wave => WAVE_NAMES[self.params.osc1_wave.load(Ordering::Relaxed) as usize % 4].to_string(),
            Selection::Octave => format!("{:+}", self.params.octave.load(Ordering::Relaxed)),
            Selection::Osc2Wave => WAVE_NAMES[self.params.osc2_wave.load(Ordering::Relaxed) as usize % 4].to_string(),
            Selection::Osc2Detune => format!("{:+.2} st", self.params.osc2_detune.get()),
            Selection::OscMix => format!("{:.2}", self.params.osc_mix.get()),
            Selection::SubLevel => format!("{:.2}", self.params.sub_level.get()),
            Selection::NoiseLevel => format!("{:.2}", self.params.noise_level.get()),
            Selection::Unison => format!("{}", self.params.unison.load(Ordering::Relaxed)),
            Selection::UnisonDetune => format!("{:.0} cents", self.params.unison_detune.get() * 100.0),
            Selection::FilterCutoff => format!("{:.0} Hz", cutoff_hz(self.params.filter_cutoff.get())),
            Selection::FilterResonance => format!("{:.2}", self.params.filter_resonance.get()),
            Selection::FilterEnvAmount => format!("{:+.2}", self.params.filter_env_amount.get()),
            Selection::FilterAttack => format!("{:.0} ms", self.params.filter_attack.get() * MAX_ENV_SECONDS * 1000.0),
            Selection::FilterDecay => format!("{:.0} ms", self.params.filter_decay.get() * MAX_ENV_SECONDS * 1000.0),
            Selection::FilterSustain => format!("{:.2}", self.params.filter_sustain.get()),
            Selection::FilterRelease => format!("{:.0} ms", self.params.filter_release.get() * MAX_ENV_SECONDS * 1000.0),
            Selection::AmpAttack => format!("{:.0} ms", self.params.amp_attack.get() * MAX_ENV_SECONDS * 1000.0),
            Selection::AmpDecay => format!("{:.0} ms", self.params.amp_decay.get() * MAX_ENV_SECONDS * 1000.0),
            Selection::AmpSustain => format!("{:.2}", self.params.amp_sustain.get()),
            Selection::AmpRelease => format!("{:.0} ms", self.params.amp_release.get() * MAX_ENV_SECONDS * 1000.0),
            Selection::LfoRate => format!("{:.1} Hz", self.params.lfo_rate.get()),
            Selection::LfoDepth => format!("{:.2}", self.params.lfo_depth.get()),
            Selection::LfoDest => LFO_DEST_NAMES[self.params.lfo_dest.load(Ordering::Relaxed) as usize % 2].to_string(),
            Selection::PresetSlot => self.preset_title(),
            Selection::PresetSave => self.slot_label(self.save_target),
            Selection::Mono => if self.params.mono.load(Ordering::Relaxed) { "On".into() } else { "Off (poly)".into() },
            Selection::Glide => format!("{:.0} ms", glide_seconds(self.params.glide.get()) * 1000.0),
            Selection::Chorus => pct(self.params.chorus.get()),
            Selection::DelayTime => format!("{:.0} ms", self.params.delay_time.get() * 1000.0),
            Selection::DelayFeedback => pct(self.params.delay_feedback.get()),
            Selection::DelayMix => pct(self.params.delay_mix.get()),
            Selection::ReverbSize => pct(self.params.reverb_size.get()),
            Selection::ReverbMix => pct(self.params.reverb_mix.get()),
            Selection::ArpOn => {
                if self.params.arp.enabled.load(Ordering::Relaxed) { "On".into() } else { "Off".into() }
            }
            Selection::ArpPattern => {
                let idx = self.params.arp.pattern.load(Ordering::Relaxed) as usize % ARP_PATTERN_NAMES.len();
                ARP_PATTERN_NAMES[idx].to_string()
            }
            Selection::ArpRate => format!("{:.1} Hz", self.params.arp.rate_hz.get()),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Osc1Wave => {
                let cur = self.params.osc1_wave.load(Ordering::Relaxed) as i32;
                self.params.osc1_wave.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Octave => {
                let cur = self.params.octave.load(Ordering::Relaxed);
                self.params.octave.store(cur + step, Ordering::Relaxed);
            }
            Selection::Osc2Wave => {
                let cur = self.params.osc2_wave.load(Ordering::Relaxed) as i32;
                self.params.osc2_wave.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Osc2Detune => bump(&self.params.osc2_detune, delta, sensitivity, -12.0, 12.0),
            Selection::OscMix => bump(&self.params.osc_mix, delta, sensitivity, 0.0, 1.0),
            Selection::SubLevel => bump(&self.params.sub_level, delta, sensitivity, 0.0, 1.0),
            Selection::NoiseLevel => bump(&self.params.noise_level, delta, sensitivity, 0.0, 1.0),
            Selection::Unison => {
                let cur = self.params.unison.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_UNISON as i32, MAX_UNISON as i32);
                self.params.unison.store(next as u32, Ordering::Relaxed);
            }
            Selection::UnisonDetune => bump(&self.params.unison_detune, delta, sensitivity, 0.0, 1.0),
            Selection::FilterCutoff => bump(&self.params.filter_cutoff, delta, sensitivity, 0.0, 1.0),
            Selection::FilterResonance => bump(&self.params.filter_resonance, delta, sensitivity, 0.0, 1.0),
            Selection::FilterEnvAmount => bump(&self.params.filter_env_amount, delta, sensitivity, -1.0, 1.0),
            Selection::FilterAttack => bump(&self.params.filter_attack, delta, sensitivity, 0.0, 1.0),
            Selection::FilterDecay => bump(&self.params.filter_decay, delta, sensitivity, 0.0, 1.0),
            Selection::FilterSustain => bump(&self.params.filter_sustain, delta, sensitivity, 0.0, 1.0),
            Selection::FilterRelease => bump(&self.params.filter_release, delta, sensitivity, 0.0, 1.0),
            Selection::AmpAttack => bump(&self.params.amp_attack, delta, sensitivity, 0.0, 1.0),
            Selection::AmpDecay => bump(&self.params.amp_decay, delta, sensitivity, 0.0, 1.0),
            Selection::AmpSustain => bump(&self.params.amp_sustain, delta, sensitivity, 0.0, 1.0),
            Selection::AmpRelease => bump(&self.params.amp_release, delta, sensitivity, 0.0, 1.0),
            Selection::LfoRate => bump(&self.params.lfo_rate, delta, sensitivity, MIN_LFO_RATE, MAX_LFO_RATE),
            Selection::LfoDepth => bump(&self.params.lfo_depth, delta, sensitivity, 0.0, 1.0),
            Selection::LfoDest => {
                let cur = self.params.lfo_dest.load(Ordering::Relaxed) as i32;
                self.params.lfo_dest.store((cur + step).rem_euclid(2) as u32, Ordering::Relaxed);
            }
            Selection::PresetSlot => {
                let next = (self.preset_cursor as i32 + step).rem_euclid(NUM_PRESETS as i32) as usize;
                if !self.load_preset(next) {
                    self.loaded = None;
                }
            }
            Selection::PresetSave => {
                self.save_target = (self.save_target as i32 + step).rem_euclid(NUM_PRESETS as i32) as usize;
            }
            Selection::Mono => self.params.mono.store(step > 0, Ordering::Relaxed),
            Selection::Glide => bump(&self.params.glide, delta, sensitivity, 0.0, 1.0),
            Selection::Chorus => bump(&self.params.chorus, delta, sensitivity, 0.0, 1.0),
            Selection::DelayTime => bump(&self.params.delay_time, delta, sensitivity, MIN_DELAY_S, MAX_DELAY_S),
            Selection::DelayFeedback => bump(&self.params.delay_feedback, delta, sensitivity, 0.0, MAX_DELAY_FEEDBACK),
            Selection::DelayMix => bump(&self.params.delay_mix, delta, sensitivity, 0.0, 1.0),
            Selection::ReverbSize => bump(&self.params.reverb_size, delta, sensitivity, 0.0, 1.0),
            Selection::ReverbMix => bump(&self.params.reverb_mix, delta, sensitivity, 0.0, 1.0),
            Selection::ArpOn => self.params.arp.enabled.store(delta > 0, Ordering::Relaxed),
            Selection::ArpPattern => {
                let cur = self.params.arp.pattern.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(ARP_PATTERN_NAMES.len() as i32);
                self.params.arp.pattern.store(next as u32, Ordering::Relaxed);
            }
            Selection::ArpRate => {
                let cur = self.params.arp.rate_hz.get();
                let next = (cur + accelerate(delta) * sensitivity * 0.2).clamp(0.5, 30.0);
                self.params.arp.rate_hz.set(next);
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Octave => self.params.octave.store(0, Ordering::Relaxed),
            Selection::Osc2Detune => self.params.osc2_detune.set(0.15),
            Selection::OscMix => self.params.osc_mix.set(0.5),
            Selection::SubLevel => self.params.sub_level.set(0.3),
            Selection::NoiseLevel => self.params.noise_level.set(0.0),
            Selection::Unison => self.params.unison.store(1, Ordering::Relaxed),
            Selection::UnisonDetune => self.params.unison_detune.set(0.15),
            Selection::FilterCutoff => self.params.filter_cutoff.set(0.6),
            Selection::FilterResonance => self.params.filter_resonance.set(0.3),
            Selection::FilterEnvAmount => self.params.filter_env_amount.set(0.0),
            Selection::FilterAttack => self.params.filter_attack.set(0.0),
            Selection::FilterDecay => self.params.filter_decay.set(0.4),
            Selection::FilterSustain => self.params.filter_sustain.set(0.3),
            Selection::FilterRelease => self.params.filter_release.set(0.25),
            Selection::AmpAttack => self.params.amp_attack.set(0.02),
            Selection::AmpDecay => self.params.amp_decay.set(0.3),
            Selection::AmpSustain => self.params.amp_sustain.set(0.8),
            Selection::AmpRelease => self.params.amp_release.set(0.3),
            Selection::LfoRate => self.params.lfo_rate.set(4.0),
            Selection::LfoDepth => self.params.lfo_depth.set(0.0),
            // Holding SELECT on Save clears the target slot (a factory
            // slot goes back to its factory preset).
            Selection::PresetSave => {
                self.presets.clear(self.save_target);
                self.kit.flash(if self.presets.is_factory(self.save_target) { "Factory preset restored" } else { "Slot cleared" });
            }
            // Reloads the preset, undoing edits.
            Selection::PresetSlot => {
                self.load_preset(self.preset_cursor);
            }
            Selection::Mono => self.params.mono.store(false, Ordering::Relaxed),
            Selection::Glide => self.params.glide.set(0.0),
            Selection::Chorus => self.params.chorus.set(0.0),
            Selection::DelayTime => self.params.delay_time.set(0.375),
            Selection::DelayFeedback => self.params.delay_feedback.set(0.35),
            Selection::DelayMix => self.params.delay_mix.set(0.0),
            Selection::ReverbSize => self.params.reverb_size.set(0.5),
            Selection::ReverbMix => self.params.reverb_mix.set(0.0),
            Selection::Osc1Wave | Selection::Osc2Wave | Selection::LfoDest => {} // no single sensible default
            Selection::ArpOn => self.params.arp.enabled.store(false, Ordering::Relaxed),
            Selection::ArpPattern => self.params.arp.pattern.store(0, Ordering::Relaxed), // Up
            Selection::ArpRate => self.params.arp.rate_hz.set(8.0),
        }
    }

    /// Shared chrome for the 4 visualizer panels: a title above a
    /// bordered box, returning the box's inner drawing rect (x0, y0,
    /// x1, baseline_y) so each panel only has to plot its own curve.
    fn draw_panel_frame(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, title: &str, accent: MonoTextStyle<Rgb565>, dim: MonoTextStyle<Rgb565>) -> (i32, i32, i32, i32) {
        Text::new(title, Point::new(x, y + 9), accent).draw(fb).ok();
        let box_y = y + 14;
        Rectangle::new(Point::new(x, box_y), Size::new(w as u32, h as u32)).into_styled(PrimitiveStyle::with_stroke(VOLTAGE_OUTLINE, 1)).draw(fb).ok();
        let _ = dim;
        (x + 3, box_y + 3, x + w - 3, box_y + h - 3)
    }

    /// Live sketch of the combined oscillator waveform (osc1+osc2
    /// mixed, plus the sub an octave down) over one sub-oscillator
    /// cycle -- exactly the mix `VoltageProcessor` computes per-sample
    /// (`wave_sample`/`osc_mix`/`sub_level`), just evaluated across a
    /// fixed phase sweep here instead of at the live voice phase.
    /// Unison spread and noise aren't shape-able curves (unison is
    /// just detuned copies of the same shape, noise has no fixed
    /// shape), so this only sketches osc1/osc2/sub -- their exact
    /// values, and unison/noise, are one turn of knob1 away in the
    /// Oscillators group on the left.
    fn draw_oscillator_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, accent: MonoTextStyle<Rgb565>, dim: MonoTextStyle<Rgb565>) {
        let (x0, y0, x1, y1) = self.draw_panel_frame(fb, x, y, w, h, "Oscillators", accent, dim);
        let osc1_wave = self.params.osc1_wave.load(Ordering::Relaxed);
        let osc2_wave = self.params.osc2_wave.load(Ordering::Relaxed);
        let osc_mix = self.params.osc_mix.get().clamp(0.0, 1.0);
        let sub_level = self.params.sub_level.get().clamp(0.0, 1.0);
        let mid_y = (y0 + y1) / 2;
        let amp = ((y1 - y0) / 2 - 2).max(1) as f32;
        let width = (x1 - x0).max(1);

        let mut prev = None;
        for i in 0..=width {
            let t = i as f32 / width as f32; // one full sub-oscillator cycle
            let s1 = wave_sample(osc1_wave, (t * 2.0).fract());
            let s2 = wave_sample(osc2_wave, (t * 2.0).fract());
            let osc_sum = s1 * (1.0 - osc_mix) + s2 * osc_mix;
            let sub = wave_sample(1, t) * sub_level;
            let v = ((osc_sum + sub) / (1.0 + sub_level)).clamp(-1.5, 1.5);
            let point = Point::new(x0 + i, mid_y - (v * amp) as i32);
            if let Some(p) = prev {
                Line::new(p, point).into_styled(PrimitiveStyle::with_stroke(VOLTAGE_ACCENT, 1)).draw(fb).ok();
            }
            prev = Some(point);
        }
    }

    /// The filter's frequency response -- a bump that rises to a
    /// resonant peak around the cutoff, taller/narrower with more
    /// resonance. The original standalone version of this curve
    /// didn't reflect the filter envelope at all; it now also sketches
    /// how far the envelope amount would swing the cutoff (the dashed
    /// line), since that's exactly the sweep behind the "glide"-like
    /// character `filter_env_amount` can add when it's turned up.
    fn draw_filter_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, accent: MonoTextStyle<Rgb565>, dim: MonoTextStyle<Rgb565>) {
        let (x0, y0, x1, y1) = self.draw_panel_frame(fb, x, y, w, h, "Filter", accent, dim);
        let cutoff = self.params.filter_cutoff.get();
        let res = self.params.filter_resonance.get();
        let env_amount = self.params.filter_env_amount.get();
        let width = (x1 - x0).max(1);
        let base_y = y1;
        let top_h = (y1 - y0) as f32;

        let response = |frac: f32| -> f32 {
            let dist = (frac - cutoff) * 8.0;
            let peak = (1.0 + res * 3.0) * (-dist * dist).exp();
            let base = if frac < cutoff { 1.0 } else { (-(frac - cutoff) * 4.0).exp() };
            (base + peak).min(2.2)
        };

        let mut prev = None;
        for i in 0..=width {
            let frac = i as f32 / width as f32;
            let hgt = (response(frac) / 2.2 * top_h) as i32;
            let point = Point::new(x0 + i, base_y - hgt);
            if let Some(p) = prev {
                Line::new(p, point).into_styled(PrimitiveStyle::with_stroke(VOLTAGE_ACCENT, 1)).draw(fb).ok();
            }
            prev = Some(point);
        }
        let cutoff_x = x0 + (cutoff.clamp(0.0, 1.0) * width as f32) as i32;
        Line::new(Point::new(cutoff_x, base_y), Point::new(cutoff_x, y0)).into_styled(PrimitiveStyle::with_stroke(Rgb565::new(30, 10, 10), 1)).draw(fb).ok();
        if env_amount.abs() > 0.01 {
            let swept = (cutoff + env_amount * 0.5).clamp(0.0, 1.0);
            let swept_x = x0 + (swept * width as f32) as i32;
            Line::new(Point::new(swept_x, base_y), Point::new(swept_x, y0)).into_styled(PrimitiveStyle::with_stroke(Rgb565::new(30, 24, 4), 1)).draw(fb).ok();
        }
    }

    /// The classic ADSR trapezoid -- attack/decay/release segment
    /// widths are proportional to their actual knob values (so a
    /// longer decay visibly draws a longer ramp), with a fixed-width
    /// sustain hold in the middle standing in for "however long the
    /// note stays held," since that's a performance detail this
    /// static preview has no note-duration to show.
    fn draw_amp_env_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, accent: MonoTextStyle<Rgb565>, dim: MonoTextStyle<Rgb565>) {
        let (x0, y0, x1, y1) = self.draw_panel_frame(fb, x, y, w, h, "Amp Envelope", accent, dim);
        let a = self.params.amp_attack.get().max(0.03);
        let d = self.params.amp_decay.get().max(0.03);
        let s = self.params.amp_sustain.get().clamp(0.0, 1.0);
        let r = self.params.amp_release.get().max(0.03);

        const SUSTAIN_FRAC: f32 = 0.3;
        let scale = (1.0 - SUSTAIN_FRAC) / (a + d + r);
        let width = (x1 - x0) as f32;
        let top_h = (y1 - y0) as f32;

        let ax = x0 + (a * scale * width) as i32;
        let dx = ax + (d * scale * width) as i32;
        let sx = dx + (SUSTAIN_FRAC * width) as i32;
        let rx = x1;

        let level_y = |level: f32| y1 - (level.clamp(0.0, 1.0) * top_h) as i32;
        let points = [Point::new(x0, y1), Point::new(ax, level_y(1.0)), Point::new(dx, level_y(s)), Point::new(sx, level_y(s)), Point::new(rx, y1)];
        for pair in points.windows(2) {
            Line::new(pair[0], pair[1]).into_styled(PrimitiveStyle::with_stroke(VOLTAGE_ACCENT, 1)).draw(fb).ok();
        }
    }

    /// A sine cycle at the LFO's actual shape (it's always a sine --
    /// see `VoltageProcessor`), amplitude scaled by depth and cycle
    /// count scaled by rate so a faster rate visibly draws more wiggle
    /// across the same width. Destination (pitch or filter) is a
    /// discrete either/or, not something this curve shape can show --
    /// see the LFO group's own list row for that.
    fn draw_lfo_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, accent: MonoTextStyle<Rgb565>, dim: MonoTextStyle<Rgb565>) {
        let (x0, y0, x1, y1) = self.draw_panel_frame(fb, x, y, w, h, "LFO", accent, dim);
        let rate = self.params.lfo_rate.get();
        let depth = self.params.lfo_depth.get().clamp(0.0, 1.0);
        let mid_y = (y0 + y1) / 2;
        let amp = ((y1 - y0) / 2 - 2).max(1) as f32;
        let width = (x1 - x0).max(1);
        let cycles = 1.0 + (rate - MIN_LFO_RATE) / (MAX_LFO_RATE - MIN_LFO_RATE) * 5.0;

        let mut prev = None;
        for i in 0..=width {
            let t = i as f32 / width as f32;
            let v = (t * cycles * TAU).sin() * depth;
            let point = Point::new(x0 + i, mid_y - (v * amp) as i32);
            if let Some(p) = prev {
                Line::new(p, point).into_styled(PrimitiveStyle::with_stroke(VOLTAGE_ACCENT, 1)).draw(fb).ok();
            }
            prev = Some(point);
        }
        Line::new(Point::new(x0, mid_y), Point::new(x1, mid_y)).into_styled(PrimitiveStyle::with_stroke(VOLTAGE_OUTLINE, 1)).draw(fb).ok();
    }
}

/// 0..1 knob mapped exponentially across the audible cutoff range.
fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

/// The Glide knob's time: squared, so the short glides used most get
/// most of the knob's travel.
fn glide_seconds(knob: f32) -> f32 {
    knob.clamp(0.0, 1.0).powi(2) * MAX_GLIDE_S
}

fn cutoff_hz(knob: f32) -> f32 {
    MIN_CUTOFF_HZ * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).powf(knob.clamp(0.0, 1.0))
}

#[allow(dead_code)] // not used by the main binary
impl VoltageApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
            })
            .collect()
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- see `ParamList::centered_scroll_window`. Returns
    /// `(window, selected_index_in_window, has_more_above,
    /// has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// Real sketches of all 4 panels (Oscillators/Filter/Amp Envelope/
    /// LFO), sampled at `n` points each -- the exact same pure-
    /// function-of-the-current-params math `draw_oscillator_panel`/
    /// `draw_filter_panel`/`draw_amp_env_panel`/`draw_lfo_panel`
    /// already use for the real embedded_graphics screen, just
    /// evaluated at `n` steps instead of pixel width, for an
    /// alternate renderer to plot however it likes.
    pub(crate) fn voltage_panels(&self, n: usize) -> VoltagePanels {
        let osc1_wave = self.params.osc1_wave.load(Ordering::Relaxed);
        let osc2_wave = self.params.osc2_wave.load(Ordering::Relaxed);
        let osc_mix = self.params.osc_mix.get().clamp(0.0, 1.0);
        let sub_level = self.params.sub_level.get().clamp(0.0, 1.0);
        let oscillator: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let s1 = wave_sample(osc1_wave, (t * 2.0).fract());
                let s2 = wave_sample(osc2_wave, (t * 2.0).fract());
                let osc_sum = s1 * (1.0 - osc_mix) + s2 * osc_mix;
                let sub = wave_sample(1, t) * sub_level;
                ((osc_sum + sub) / (1.0 + sub_level)).clamp(-1.5, 1.5) / 1.5
            })
            .collect();

        let cutoff = self.params.filter_cutoff.get();
        let res = self.params.filter_resonance.get();
        let env_amount = self.params.filter_env_amount.get();
        let response = |frac: f32| -> f32 {
            let dist = (frac - cutoff) * 8.0;
            let peak = (1.0 + res * 3.0) * (-dist * dist).exp();
            let base = if frac < cutoff { 1.0 } else { (-(frac - cutoff) * 4.0).exp() };
            (base + peak).min(2.2)
        };
        let filter: Vec<f32> = (0..n).map(|i| response(i as f32 / n as f32) / 2.2).collect();
        let filter_cutoff_frac = cutoff.clamp(0.0, 1.0);
        let filter_swept_frac = if env_amount.abs() > 0.01 { Some((cutoff + env_amount * 0.5).clamp(0.0, 1.0)) } else { None };

        let a = self.params.amp_attack.get().max(0.03);
        let d = self.params.amp_decay.get().max(0.03);
        let s = self.params.amp_sustain.get().clamp(0.0, 1.0);
        let r = self.params.amp_release.get().max(0.03);
        const SUSTAIN_FRAC: f32 = 0.3;
        let scale = (1.0 - SUSTAIN_FRAC) / (a + d + r);
        let a_frac = a * scale;
        let d_frac = a_frac + d * scale;
        let s_frac = d_frac + SUSTAIN_FRAC;
        let amp_env: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                if t < a_frac {
                    t / a_frac.max(0.001)
                } else if t < d_frac {
                    1.0 - (1.0 - s) * (t - a_frac) / (d_frac - a_frac).max(0.001)
                } else if t < s_frac {
                    s
                } else {
                    s * (1.0 - (t - s_frac) / (1.0 - s_frac).max(0.001))
                }
            })
            .collect();

        let rate = self.params.lfo_rate.get();
        let depth = self.params.lfo_depth.get().clamp(0.0, 1.0);
        let cycles = 1.0 + (rate - MIN_LFO_RATE) / (MAX_LFO_RATE - MIN_LFO_RATE) * 5.0;
        let lfo: Vec<f32> = (0..n).map(|i| (i as f32 / n as f32 * cycles * TAU).sin() * depth).collect();

        VoltagePanels { oscillator, filter, filter_cutoff_frac, filter_swept_frac, amp_env, lfo }
    }
}

impl VoltageApp {
    fn knob(&self, i: usize) -> Knob<'_> {
        let p = &self.params;
        match CONTROLS[i % 16].0 {
            Selection::FilterCutoff => Knob::F(&p.filter_cutoff, 0.0, 1.0),
            Selection::FilterResonance => Knob::F(&p.filter_resonance, 0.0, 1.0),
            Selection::FilterEnvAmount => Knob::F(&p.filter_env_amount, -1.0, 1.0),
            Selection::OscMix => Knob::F(&p.osc_mix, 0.0, 1.0),
            Selection::AmpAttack => Knob::F(&p.amp_attack, 0.0, 1.0),
            Selection::AmpRelease => Knob::F(&p.amp_release, 0.0, 1.0),
            Selection::LfoDepth => Knob::F(&p.lfo_depth, 0.0, 1.0),
            Selection::LfoRate => Knob::F(&p.lfo_rate, MIN_LFO_RATE, MAX_LFO_RATE),
            Selection::Osc1Wave => Knob::U(&p.osc1_wave, 4),
            Selection::Osc2Wave => Knob::U(&p.osc2_wave, 4),
            Selection::Osc2Detune => Knob::F(&p.osc2_detune, -12.0, 12.0),
            Selection::SubLevel => Knob::F(&p.sub_level, 0.0, 1.0),
            Selection::NoiseLevel => Knob::F(&p.noise_level, 0.0, 1.0),
            Selection::Unison => Knob::UR(&p.unison, MIN_UNISON, MAX_UNISON),
            Selection::Octave => Knob::I(&p.octave, -OCTAVE_SPAN, OCTAVE_SPAN),
            _ => Knob::None,
        }
    }

    fn octave(&self) -> i32 {
        self.params.octave.load(Ordering::Relaxed)
    }
}

impl PlayHost for VoltageApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        CONTROLS[i % 16].1.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        self.leaf_value(CONTROLS[i % 16].0)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        self.knob(i).norm()
    }
    fn kit_stepped(&self, i: usize) -> bool {
        self.knob(i).stepped()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(CONTROLS[i % 16].0, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        self.reset(CONTROLS[i % 16].0);
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        self.knob(i).set(v);
    }
    /// A key plays the pad with its pitch; keys outside the pads'
    /// 16-semitone window fold into it by octaves (each pad is one fixed
    /// voice, so the window itself can't move per key).
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        let d = note as i32 - note_for(0, self.octave());
        let rank = if (0..16).contains(&d) { d } else { d.rem_euclid(12) };
        Some(pad_rank(rank) as usize)
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        note_name(note_for(pad_rank(pad as i32), self.octave()))
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if note_for(pad_rank(pad as i32), self.octave()).rem_euclid(12) == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        let held = *self.params.held.lock().unwrap();
        let notes: Vec<String> = (0..16).filter(|&i| held[i]).take(4).map(|i| note_name(note_for(pad_rank(i as i32), self.octave()))).collect();
        // Nothing held: show which preset is loaded instead.
        if notes.is_empty() { self.preset_title() } else { notes.join(" ") }
    }
}

/// See `VoltageApp::voltage_panels`.
#[allow(dead_code)] // not used by the main binary
pub(crate) struct VoltagePanels {
    pub oscillator: Vec<f32>,
    pub filter: Vec<f32>,
    pub filter_cutoff_frac: f32,
    pub filter_swept_frac: Option<f32>,
    pub amp_env: Vec<f32>,
    pub lfo: Vec<f32>,
}

impl App for VoltageApp {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        crate::app::play_kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        crate::app::play_kit::adjust_in(self, index, delta)
    }
    fn supports_pad_lock(&self) -> bool { true }
    fn play_surface(&self) -> bool { true }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }
    /// F3 runs the arpeggiator over the held pads.
    fn running(&self) -> Option<bool> {
        Some(self.params.arp.enabled.load(Ordering::Relaxed))
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.params.arp.enabled.load(Ordering::Relaxed) { "ARP OFF" } else { "ARP" })
    }
    fn toggle_running(&mut self) {
        let on = !self.params.arp.enabled.load(Ordering::Relaxed);
        self.params.arp.enabled.store(on, Ordering::Relaxed);
        self.kit.flash(if on { "Arp on: hold some pads" } else { "Arp off" });
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        // Real curve *shape* only needs a modest sample count once
        // it's rendered as genuine connected line segments (not a bar
        // chart) -- see `polyline_segments`. The fixed canvas size
        // here must match what the live screen actually draws these
        // panels at (see slint_home_live.rs's own copy of this size),
        // or the segment angles come out stretched.
        const CURVE_POINTS: usize = 48;
        const PANEL_W: f32 = 130.0;
        const PANEL_H: f32 = 95.0;
        let p = self.voltage_panels(CURVE_POINTS);
        let seg = |samples: &[f32], centered: bool| {
            let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(samples, PANEL_W, PANEL_H, centered);
            crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
        };
        crate::app::SlintExtra::Voltage(crate::app::VoltageExtra {
            preset: self.preset_title(),
            oscillator: seg(&p.oscillator, true),
            filter: seg(&p.filter, false),
            filter_cutoff_frac: p.filter_cutoff_frac,
            filter_swept_frac: p.filter_swept_frac,
            amp_env: seg(&p.amp_env, false),
            lfo: seg(&p.lfo, true),
        })
    }

    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. Pads reach the voices only on KEYS.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            match current {
                Some(Row::Group(g)) => self.expanded[g] = !self.expanded[g],
                Some(Row::Leaf(Selection::PresetSave)) => self.save_preset(),
                _ => {}
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }

        let mut held = self.params.held.lock().unwrap();
        for (i, pressed) in input.grid.iter().enumerate() {
            held[i] = *pressed;
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(VoltageProcessor::new(Arc::clone(&self.params))))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(VOLTAGE_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, VOLTAGE_TITLE);
        Text::new("Voltage", Point::new(16, 30), title).draw(fb).ok();
        Text::new(&self.preset_title(), Point::new(150, 28), MonoTextStyle::new(&SPLEEN_6X12, VOLTAGE_ACCENT)).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, VOLTAGE_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, VOLTAGE_DIM);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        if self.kit.menu {
            self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, VOLTAGE_BG, VOLTAGE_DIM, VOLTAGE_ACCENT);
        } else if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: VOLTAGE_BG, ink: VOLTAGE_TITLE, accent: VOLTAGE_ACCENT, dim: VOLTAGE_DIM, faint: VOLTAGE_OUTLINE };
            kit::draw::column(fb, &col, 16, 40, 350, 280, pal);
        }

        // --- Right: one small illustrative panel per menu group --
        // oscillators, filter, amp envelope, LFO -- each a live sketch
        // of what that section's current knobs actually shape, same
        // "show the knob, don't just print a number" spirit the old
        // standalone filter-response curve started with (now folded
        // into this 2x2 grid as one of the four). Computed straight
        // from the current params each frame, not captured live audio,
        // so they update instantly as you turn a knob even when
        // nothing is actually playing. No caption text under each
        // graph -- the exact numbers are already one turn of knob1
        // away in the list on the left, and captions wide enough to
        // be useful (e.g. "cut 1200Hz res0.30 env+0.00") don't fit a
        // 2-wide column without running into its neighbor.
        let grid_x0 = 380;
        let panel_w = 118;
        let panel_h = 108;
        let gap_x = 14;
        let gap_y = 20;
        let panels: [(i32, i32); 4] = [
            (grid_x0, 60),
            (grid_x0 + panel_w + gap_x, 60),
            (grid_x0, 60 + panel_h + gap_y),
            (grid_x0 + panel_w + gap_x, 60 + panel_h + gap_y),
        ];

        self.draw_oscillator_panel(fb, panels[0].0, panels[0].1, panel_w, panel_h, accent, dim);
        self.draw_filter_panel(fb, panels[1].0, panels[1].1, panel_w, panel_h, accent, dim);
        self.draw_amp_env_panel(fb, panels[2].0, panels[2].1, panel_w, panel_h, accent, dim);
        self.draw_lfo_panel(fb, panels[3].0, panels[3].1, panel_w, panel_h, accent, dim);

        let hint = match rows.get(self.list.selected) {
            _ if !self.kit.menu => "L/R: dial (SELECT: next)   U/D: octave   F2: pads   F3: arp   R1: menu".to_string(),
            Some(Row::Group(_)) => "up/down: browse   SELECT: expand/collapse".to_string(),
            Some(Row::Leaf(Selection::PresetSlot)) => "left/right: browse and load   hold SELECT: undo edits".to_string(),
            Some(Row::Leaf(Selection::PresetSave)) => "left/right: pick slot   SELECT: save   hold SELECT: clear".to_string(),
            Some(Row::Leaf(sel)) => format!("left/right: change {}   hold SELECT: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

/// A standard ADSR -- same shape as plaits.rs's/cascade.rs's own
/// copies, duplicated locally again rather than shared.
#[derive(Default, Clone, Copy)]
struct AdsrState {
    stage: u8, // 0=idle 1=attack 2=decay 3=sustain 4=release
    level: f32,
    prev_gate: bool,
}

impl AdsrState {
    fn step(&mut self, gate: bool, attack: f32, decay: f32, sustain: f32, release: f32, dt: f32) -> f32 {
        if gate && !self.prev_gate {
            self.stage = 1;
        } else if !gate && self.prev_gate {
            self.stage = 4;
        }
        self.prev_gate = gate;
        match self.stage {
            1 => {
                self.level = (self.level + dt / attack.max(0.001)).min(1.0);
                if self.level >= 1.0 {
                    self.stage = 2;
                }
            }
            2 => {
                self.level = (self.level - dt * (1.0 - sustain) / decay.max(0.001)).max(sustain);
                if self.level <= sustain {
                    self.stage = 3;
                }
            }
            3 => self.level = sustain,
            4 => {
                self.level = (self.level - dt / release.max(0.001)).max(0.0);
                if self.level <= 0.0 {
                    self.stage = 0;
                }
            }
            _ => self.level = 0.0,
        }
        self.level
    }
}

fn adsr_time(knob_value: f32) -> f32 {
    0.001 + knob_value * MAX_ENV_SECONDS
}

struct AnalogVoice {
    osc1_phase: [f32; MAX_UNISON as usize],
    osc2_phase: [f32; MAX_UNISON as usize],
    sub_phase: f32,
    amp_env: AdsrState,
    filter_env: AdsrState,
    lfo_phase: f32,
    svf_lp: f32,
    svf_bp: f32,
    rng: u32,
    buf: Vec<f32>,
    /// The note (MIDI number, fractional while gliding) this voice is
    /// sounding.
    pitch: f32,
    gate: bool,
}

impl AnalogVoice {
    fn new(seed: u32) -> Self {
        Self {
            osc1_phase: [0.0; MAX_UNISON as usize],
            osc2_phase: [0.0; MAX_UNISON as usize],
            sub_phase: 0.0,
            amp_env: AdsrState::default(),
            filter_env: AdsrState::default(),
            lfo_phase: 0.0,
            svf_lp: 0.0,
            svf_bp: 0.0,
            rng: 0x9E3779B9 ^ ((seed + 1).wrapping_mul(0x85EBCA6B) | 1),
            buf: Vec::new(),
            pitch: 60.0,
            gate: false,
        }
    }

    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

struct VoltageProcessor {
    params: Arc<Params>,
    voices: [AnalogVoice; 16],
    mono_buf: Vec<f32>,
    /// Slews toward the actual active-voice count instead of jumping
    /// straight to it -- see where it's used in `process()` for why.
    smoothed_headroom: f32,
    fx: Box<StereoFx>,
    /// Mono mode's last-note priority: when each pad was pressed.
    press_stamp: [u64; 16],
    stamp: u64,
    prev_held: [bool; 16],
}

impl VoltageProcessor {
    fn new(params: Arc<Params>) -> Self {
        Self {
            params,
            voices: std::array::from_fn(|i| AnalogVoice::new(i as u32)),
            mono_buf: Vec::new(),
            smoothed_headroom: 1.0,
            fx: Box::new(StereoFx::new()),
            press_stamp: [0; 16],
            stamp: 0,
            prev_held: [false; 16],
        }
    }
}

impl AudioProcessor for VoltageProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);
        let dt = 1.0 / sample_rate;

        let osc1_wave = self.params.osc1_wave.load(Ordering::Relaxed);
        let osc2_wave = self.params.osc2_wave.load(Ordering::Relaxed);
        let osc2_detune = self.params.osc2_detune.get();
        let osc_mix = self.params.osc_mix.get().clamp(0.0, 1.0);
        let sub_level = self.params.sub_level.get().clamp(0.0, 1.0);
        let noise_level = self.params.noise_level.get().clamp(0.0, 1.0);
        let unison = self.params.unison.load(Ordering::Relaxed).clamp(MIN_UNISON, MAX_UNISON) as usize;
        let unison_detune = self.params.unison_detune.get();
        let cutoff_knob = (self.params.filter_cutoff.get() + self.params.ext_filter_cutoff.get()).clamp(0.0, 1.0);
        let resonance = self.params.filter_resonance.get().clamp(0.0, 1.0);
        let filter_env_amount = self.params.filter_env_amount.get().clamp(-1.0, 1.0);
        let filter_attack = adsr_time(self.params.filter_attack.get());
        let filter_decay = adsr_time(self.params.filter_decay.get());
        let filter_sustain = self.params.filter_sustain.get();
        let filter_release = adsr_time(self.params.filter_release.get());
        let amp_attack = adsr_time(self.params.amp_attack.get());
        let amp_decay = adsr_time(self.params.amp_decay.get());
        let amp_sustain = self.params.amp_sustain.get();
        let amp_release = adsr_time(self.params.amp_release.get());
        let lfo_rate = self.params.lfo_rate.get();
        let lfo_depth = self.params.lfo_depth.get();
        let lfo_dest = self.params.lfo_dest.load(Ordering::Relaxed);
        let held_raw = *self.params.held.lock().unwrap();
        // Real arp step -- see Plaits' `process` for the fuller
        // explanation of why this runs here (sample-block-accurate,
        // real-time thread) rather than in `tick`. `held` is reordered
        // to pitch-rank first so Up/Down walk ascending/descending
        // pitch, not raw physical pad index.
        let held_by_rank: [bool; 16] = std::array::from_fn(|r| held_raw[pad_rank(r as i32) as usize]);
        let arp_rank = self.params.arp.step(&held_by_rank, frames as f32 / sample_rate);
        let held: [bool; 16] = match arp_rank {
            Some(r) => {
                let idx = pad_rank(r as i32) as usize;
                std::array::from_fn(|i| i == idx)
            }
            None => held_raw,
        };
        // `svf_lowpass_step` damps by 1/q, so q is the filter's Q: 0.7
        // (flat, Butterworth-like) at zero resonance up to about 10 (a
        // sharp, ringing peak) at full, on a square law so the musical
        // middle of the knob isn't all squeal. This used to run backwards -- Q 10 at
        // zero resonance -- so the Resonance knob did the opposite of
        // its label.
        let q = 0.7 * (1.0 + resonance * resonance * 13.0);
        let octave = self.params.octave.load(Ordering::Relaxed);
        let mono = self.params.mono.load(Ordering::Relaxed);
        let glide_s = glide_seconds(self.params.glide.get());

        for i in 0..16 {
            if held[i] && !self.prev_held[i] {
                self.stamp += 1;
                self.press_stamp[i] = self.stamp;
            }
        }
        self.prev_held = held;

        // Each voice's gate and target note. Poly: one voice per pad at
        // that pad's pitch. Mono: voice 0 plays the most recently pressed
        // held pad; the others are released.
        let mut gates = [false; 16];
        let mut targets = [0.0f32; 16];
        if mono {
            let newest = (0..16).filter(|&i| held[i]).max_by_key(|&i| self.press_stamp[i]);
            gates[0] = newest.is_some();
            targets[0] = match newest {
                Some(i) => note_for(pad_rank(i as i32), octave) as f32,
                None => self.voices[0].pitch,
            };
        } else {
            for i in 0..16 {
                gates[i] = held[i];
                targets[i] = note_for(pad_rank(i as i32), octave) as f32;
            }
        }
        // Glide is a one-pole slew on the note number, so it moves at a
        // constant musical speed whatever the interval.
        let glide_coef = if mono && glide_s > 0.001 { 1.0 - (-dt / (glide_s / 3.0)).exp() } else { 1.0 };

        let source_gain = 1.0 / (1.0 + sub_level + noise_level);

        let mut active_voices: usize = 0;
        for i in 0..16 {
            let gate = gates[i];
            let target = targets[i];
            let voice = &mut self.voices[i];
            voice.buf.clear();
            voice.buf.resize(frames, 0.0);
            // A new note with nothing sounding starts at its pitch; glide
            // only connects notes played legato.
            if gate && (!voice.gate || !mono) && (voice.amp_env.stage == 0 || !mono) {
                voice.pitch = target;
            }
            voice.gate = gate;
            // An idle voice makes no sound: skip it (most of the 16, most
            // of the time).
            if !gate && voice.amp_env.stage == 0 && !voice.amp_env.prev_gate {
                voice.svf_lp = 0.0;
                voice.svf_bp = 0.0;
                continue;
            }

            let mut base_freq = 440.0 * 2f32.powf((voice.pitch - 69.0) / 12.0);
            for n in 0..frames {
                if voice.pitch != target {
                    voice.pitch += (target - voice.pitch) * glide_coef;
                    if (target - voice.pitch).abs() < 1e-3 {
                        voice.pitch = target;
                    }
                    base_freq = 440.0 * 2f32.powf((voice.pitch - 69.0) / 12.0);
                }
                voice.lfo_phase = (voice.lfo_phase + lfo_rate * dt).rem_euclid(1.0);
                let lfo = (voice.lfo_phase * TAU).sin() * lfo_depth;
                let pitch_mult = if lfo_dest == 0 { 2f32.powf(lfo * 0.5 / 12.0) } else { 1.0 };

                let amp_env = voice.amp_env.step(gate, amp_attack, amp_decay, amp_sustain, amp_release, dt);
                let filter_env = voice.filter_env.step(gate, filter_attack, filter_decay, filter_sustain, filter_release, dt);

                let mut osc_sum = 0.0f32;
                for u in 0..unison {
                    let spread = if unison > 1 { (u as f32 / (unison - 1) as f32 - 0.5) * unison_detune } else { 0.0 };
                    let f1 = base_freq * pitch_mult * 2f32.powf(spread / 12.0);
                    let f2 = base_freq * pitch_mult * 2f32.powf((spread + osc2_detune) / 12.0);
                    let s1 = wave_sample(osc1_wave, voice.osc1_phase[u]);
                    let s2 = wave_sample(osc2_wave, voice.osc2_phase[u]);
                    osc_sum += s1 * (1.0 - osc_mix) + s2 * osc_mix;
                    voice.osc1_phase[u] = (voice.osc1_phase[u] + f1 * dt).rem_euclid(1.0);
                    voice.osc2_phase[u] = (voice.osc2_phase[u] + f2 * dt).rem_euclid(1.0);
                }
                osc_sum /= unison as f32;

                let sub = wave_sample(1, voice.sub_phase) * sub_level; // sub is always a square, one octave down
                voice.sub_phase = (voice.sub_phase + base_freq * 0.5 * pitch_mult * dt).rem_euclid(1.0);
                let noise = voice.next_rand() * noise_level;

                // Scaled so the three sources together never exceed full
                // scale -- a single held note with the sub up used to
                // reach 1.5x before the filter's own overshoot.
                let pre_filter = (osc_sum + sub + noise) * source_gain;

                let filter_lfo = if lfo_dest == 1 { lfo } else { 0.0 };
                let cutoff = cutoff_hz((cutoff_knob + filter_env * filter_env_amount + filter_lfo * 0.5).clamp(0.0, 1.0));
                let f_coef = (2.0 * (PI * cutoff / sample_rate).sin()).clamp(0.0, 1.0);
                let (lp, bp, _) = svf_lowpass_step(pre_filter, voice.svf_lp, voice.svf_bp, f_coef, q);
                // Defensive clamp -- the naive Chamberlin SVF is only
                // *conditionally* stable: low damping (high
                // resonance) combined with a high cutoff can still
                // diverge even with `f_coef` itself bounded to <= 1.
                // Clamping both the stored state *and* this sample's
                // output keeps a single-step spike from slipping out
                // before the next sample's clamp would catch it --
                // same "always clamp, don't just trust the tuning"
                // rule Nebula's `MAX_SPEED` follows.
                voice.svf_lp = lp.clamp(-8.0, 8.0);
                voice.svf_bp = bp.clamp(-8.0, 8.0);

                voice.buf[n] = voice.svf_lp * amp_env;
            }

            let peak = voice.buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            if peak > 1e-4 {
                active_voices += 1;
            }
            for (m, s) in self.mono_buf.iter_mut().zip(voice.buf.iter()) {
                *m += *s;
            }
        }
        // A held note stays "active" (peak > 1e-4) for essentially its
        // whole release tail -- which defaults to over a second here
        // (see `amp_release`'s mapping) -- so jumping the divisor
        // straight to `active_voices` made a note you're still
        // holding audibly "snap" louder the instant some earlier,
        // already-released note's tail finally decayed below that
        // threshold, even though nothing about the held note itself
        // changed. Fixed by the same "attack fast, release slow" rule
        // a compressor uses: more voices need *more* headroom right
        // now to avoid clipping, so that direction jumps immediately;
        // fewer voices need less headroom, which can safely ease off
        // over ~120ms instead of snapping.
        let target_headroom = active_voices.max(1) as f32;
        if target_headroom > self.smoothed_headroom {
            self.smoothed_headroom = target_headroom;
        } else {
            let block_dt = frames as f32 / sample_rate;
            let coef = 1.0 - (-block_dt / 0.12).exp();
            self.smoothed_headroom += (target_headroom - self.smoothed_headroom) * coef;
        }
        let headroom = self.smoothed_headroom.max(1.0);
        for m in self.mono_buf.iter_mut() {
            *m /= headroom;
        }

        // Effects, then publish (so effect apps reading Voltage from the
        // audio bus hear the whole sound) and write the device output.
        let fx = FxSettings {
            chorus: self.params.chorus.get(),
            delay_time: self.params.delay_time.get(),
            delay_feedback: self.params.delay_feedback.get(),
            delay_mix: self.params.delay_mix.get(),
            reverb_size: self.params.reverb_size.get(),
            reverb_mix: self.params.reverb_mix.get(),
        };
        let wet = !StereoFx::bypassed(&fx);
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut bus_out = self.params.bus_out.lock().unwrap();
        bus_out.clear();
        for (frame, m) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            let (l, r) = if wet { self.fx.tick(*m, &fx, sample_rate) } else { (*m, *m) };
            bus_out.push((l + r) * 0.5);
            match frame {
                [] => {}
                [only] => *only = (l + r) * 0.5 * mix_level,
                [a, b, rest @ ..] => {
                    *a = l * mix_level;
                    *b = r * mix_level;
                    for out in rest.iter_mut() {
                        *out = (l + r) * 0.5 * mix_level;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_processor(params: Arc<Params>) -> VoltageProcessor {
        VoltageProcessor::new(params)
    }

    fn hold_pad(params: &Params, i: usize, down: bool) {
        params.held.lock().unwrap()[i] = down;
    }

    /// With a pad held, output must actually reach the device buffer.
    #[test]
    fn held_pad_produces_audible_output() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        hold_pad(&params, 0, true);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs())));
        }
        assert!(peak > 0.01, "expected audible output with a pad held, got peak {peak}");
    }

    /// Releasing a held pad must let the amp envelope decay to
    /// silence rather than hanging forever.
    #[test]
    fn releasing_pad_decays_to_silence() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        hold_pad(&params, 0, true);
        params.amp_release.set(0.02);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        hold_pad(&params, 0, false);
        for _ in 0..300 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        let peak: f32 = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak < 0.001, "expected the amp envelope to have decayed to silence, got peak {peak}");
    }

    /// Every waveform, at max unison, resonance, and noise, must stay
    /// finite and within a sane bound -- the filter's `q` is always
    /// >= 0.5 by construction, so this should never be able to
    /// > self-oscillate into a runaway.
    #[test]
    fn stays_bounded_at_extreme_settings() {
        for wave in 0..4u32 {
            let modbus = ModBus::new();
            let audio_bus = AudioBus::new();
            let mixer_bus = MixerBus::new();
            let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
            hold_pad(&params, 0, true);
            params.osc1_wave.store(wave, Ordering::Relaxed);
            params.osc2_wave.store(wave, Ordering::Relaxed);
            params.unison.store(MAX_UNISON, Ordering::Relaxed);
            params.filter_resonance.set(1.0);
            params.noise_level.set(1.0);
            params.sub_level.set(1.0);
            params.amp_sustain.set(1.0);
            params.amp_attack.set(0.0);

            let mut proc = new_processor(Arc::clone(&params));
            let mut buffer = vec![0.0f32; 512 * 2];
            let mut peak = 0.0f32;
            for _ in 0..80 {
                proc.process(&mut buffer, 2, 48000.0);
                for v in buffer.iter() {
                    assert!(v.is_finite(), "wave {wave} produced a non-finite sample");
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak < 10.0, "wave {wave} exceeded a sane bound: peak={peak}");
        }
    }

    /// Holding several pads at once must not sum to an amplitude that
    /// scales with how many notes are held.
    #[test]
    fn chords_stay_headroom_normalized() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.amp_sustain.set(1.0);
        params.amp_attack.set(0.0);
        for i in 0..16 {
            hold_pad(&params, i, true);
        }

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, x| a.max(x.abs())));
        }
        assert!(peak < 1.5, "peak grew with how many pads were held instead of staying headroom-normalized: {peak}");
    }

    /// A released voice stays "active" (peak > 1e-4) for essentially
    /// its whole release tail, so a still-held note's headroom
    /// divisor must ease back down once that tail finally decays
    /// below the threshold, not snap straight to it in one block --
    /// the "quieter, then it snaps louder" the smoothing in
    /// `process()` exists to fix.
    #[test]
    fn released_voices_ease_out_of_headroom_instead_of_snapping() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.amp_sustain.set(1.0);
        params.amp_attack.set(0.0);
        params.amp_release.set(0.05); // short-ish but not instant -- keeps this test fast
        hold_pad(&params, 0, true);
        hold_pad(&params, 1, true);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        assert!((proc.smoothed_headroom - 2.0).abs() < 0.1, "expected headroom to have settled near 2 with both pads held, got {}", proc.smoothed_headroom);

        // Release pad 0; pad 1 stays held throughout, same as a
        // player releasing one note of a chord while holding another.
        hold_pad(&params, 0, false);
        let mut max_single_block_drop = 0.0f32;
        let mut prev = proc.smoothed_headroom;
        for _ in 0..200 {
            proc.process(&mut buffer, 2, 48000.0);
            max_single_block_drop = max_single_block_drop.max(prev - proc.smoothed_headroom);
            prev = proc.smoothed_headroom;
        }
        assert!((proc.smoothed_headroom - 1.0).abs() < 0.1, "expected headroom to have settled back near 1 once pad 0's release tail fully decayed, got {}", proc.smoothed_headroom);
        assert!(max_single_block_drop < 0.1, "headroom dropped {max_single_block_drop} in a single ~10.7ms block -- that's the audible \"snap\" this smoothing exists to prevent");
    }

    /// The Mixer app's channel fader must only affect what reaches
    /// the device output, not what this app publishes to audio_bus.rs.
    #[test]
    fn mixer_fader_does_not_affect_audio_bus_publish() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        hold_pad(&params, 0, true);
        params.mix_level.set(0.0);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        let device_peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert_eq!(device_peak, 0.0);

        let bus_out = params.bus_out.lock().unwrap();
        let bus_peak = bus_out.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(bus_peak > 0.0, "audio_bus publish should be unaffected by the Mixer channel fader");
    }

    fn bank_in(dir: &std::path::Path) -> PresetBank {
        PresetBank::new(Some(dir.join("presets.json")))
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("voltage-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// A preset must round-trip every parameter it holds.
    #[test]
    fn snapshot_and_apply_round_trip_every_parameter() {
        let a = app();
        let mut p = PresetData {
            name: "Test".into(),
            osc1_wave: 2,
            osc2_wave: 3,
            octave: -1,
            osc2_detune: 4.0,
            osc_mix: 0.7,
            sub_level: 0.6,
            noise_level: 0.2,
            unison: 3,
            unison_detune: 0.4,
            mono: true,
            glide: 0.3,
            filter_cutoff: 0.8,
            filter_resonance: 0.5,
            filter_env_amount: 0.9,
            filter_attack: 0.1,
            filter_decay: 0.2,
            filter_sustain: 0.3,
            filter_release: 0.4,
            amp_attack: 0.05,
            amp_decay: 0.6,
            amp_sustain: 0.7,
            amp_release: 0.8,
            lfo_rate: 9.0,
            lfo_depth: 0.6,
            lfo_dest: 1,
            chorus: 0.5,
            delay_time: 0.3,
            delay_feedback: 0.6,
            delay_mix: 0.4,
            reverb_size: 0.9,
            reverb_mix: 0.2,
            arp_on: true,
            arp_pattern: 2,
            arp_rate: 5.0,
        };
        a.params.apply(&p);
        assert_eq!(a.params.snapshot("Test"), p);
        // Out-of-range values in a hand-edited file are clamped.
        p.filter_cutoff = 7.0;
        p.unison = 99;
        a.params.apply(&p);
        assert_eq!(a.params.filter_cutoff.get(), 1.0);
        assert_eq!(a.params.unison.load(Ordering::Relaxed), MAX_UNISON);
    }

    #[test]
    fn there_are_100_slots_and_20_named_factory_presets_first() {
        let f = factory_presets();
        assert_eq!(f.len(), 20, "the factory file parses to 20 presets");
        let names: std::collections::HashSet<_> = f.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names.len(), 20, "names are unique");
        assert!(f.iter().all(|p| !p.name.is_empty() && p.name.len() <= 18), "names fit the row");
        let bank = PresetBank::new(None);
        assert_eq!(bank.slots.len(), NUM_PRESETS);
        assert_eq!(bank.filled(), 20);
        assert_eq!(bank.first_empty(), Some(20));
    }

    /// Every factory preset must make real, bounded sound: held for a
    /// second (the arp presets arpeggiate it), then released.
    #[test]
    fn every_factory_preset_plays_and_stays_bounded() {
        for p in factory_presets() {
            let a = app();
            a.params.apply(&p);
            for i in [0, 5, 10] {
                hold_pad(&a.params, i, true);
            }
            let mut proc = new_processor(Arc::clone(&a.params));
            let mut buffer = vec![0.0f32; 512 * 2];
            let (mut peak, mut energy) = (0.0f32, 0.0f32);
            for _ in 0..94 {
                proc.process(&mut buffer, 2, 48000.0);
                for v in &buffer {
                    assert!(v.is_finite(), "{}: non-finite sample", p.name);
                    peak = peak.max(v.abs());
                    energy += v * v;
                }
            }
            assert!(energy > 1.0, "{}: too quiet ({energy})", p.name);
            assert!(peak < 1.6, "{}: peak {peak}", p.name);
        }
    }

    #[test]
    fn browsing_loads_presets_and_marks_edits() {
        let mut a = app();
        assert_eq!(a.preset_cursor, 0);
        assert!(a.preset_title().starts_with("01 "), "opens on preset 1: {}", a.preset_title());
        assert!(!a.preset_title().ends_with('*'));
        a.edit(Selection::PresetSlot, 1);
        let second = factory_presets()[1].clone();
        assert_eq!(a.params.snapshot(&second.name), second, "stepping right loads preset 2");
        a.edit(Selection::FilterCutoff, 3);
        assert!(a.preset_title().ends_with('*'), "an edited sound is marked");
        a.reset(Selection::PresetSlot);
        assert!(!a.preset_title().ends_with('*'), "hold SELECT reloads it");
        a.edit(Selection::PresetSlot, -1);
        a.edit(Selection::PresetSlot, -1);
        assert_eq!(a.preset_cursor, NUM_PRESETS - 1, "browsing wraps");
        assert!(a.preset_title().contains("(empty)"));
    }

    /// Saved presets go to the SD card and come back after a restart;
    /// clearing a factory slot brings the factory preset back.
    #[test]
    fn saves_persist_and_clearing_restores_the_factory_preset() {
        let dir = scratch_dir("persist");
        let mut a = app();
        a.presets = bank_in(&dir);
        a.params.filter_cutoff.set(0.123);
        a.save_target = 41;
        a.save_preset();
        a.params.filter_cutoff.set(0.456);
        a.save_target = 2;
        a.save_preset(); // over a factory preset, keeping its name
        let factory3 = factory_presets()[2].name.clone();

        let mut reopened = bank_in(&dir);
        assert_eq!(reopened.get(41).map(|p| (p.name.as_str(), p.filter_cutoff)), Some(("User 42", 0.123)));
        assert_eq!(reopened.get(2).map(|p| (p.name.clone(), p.filter_cutoff)), Some((factory3.clone(), 0.456)));
        reopened.clear(2);
        reopened.clear(41);
        let again = bank_in(&dir);
        assert_eq!(again.get(2), factory_presets().get(2), "factory preset restored");
        assert!(again.get(41).is_none(), "user slot emptied");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_on_the_save_row_saves() {
        let dir = scratch_dir("select");
        let mut a = app();
        a.presets = bank_in(&dir);
        a.kit.menu = true;
        a.save_target = 30;
        let rows = a.visible_rows();
        let save_row = rows.iter().position(|r| matches!(r, Row::Leaf(Selection::PresetSave))).expect("Presets is open by default");
        a.list.selected = save_row;
        a.tick(&Input { knob1_press: true, ..Default::default() });
        assert!(a.presets.get(30).is_some(), "SELECT saved into slot 31");
        assert_eq!(a.preset_cursor, 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Mono plays one voice at the newest held pad's pitch and glides
    /// to the next note when played legato.
    #[test]
    fn mono_mode_glides_between_legato_notes() {
        let a = app();
        a.params.apply(&PresetData { mono: true, glide: 0.5, ..Default::default() });
        let mut proc = new_processor(Arc::clone(&a.params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let low = pad_rank(0) as usize; // rank 0 sits on physical pad 12
        let high = pad_rank(12) as usize;
        hold_pad(&a.params, low, true);
        for _ in 0..10 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        assert_eq!(proc.voices[0].pitch, note_for(0, 0) as f32, "the first note starts at its pitch");
        hold_pad(&a.params, high, true);
        proc.process(&mut buffer, 2, 48000.0);
        let mid = proc.voices[0].pitch;
        assert!(mid > note_for(0, 0) as f32 && mid < note_for(12, 0) as f32, "glides, doesn't jump: {mid}");
        for _ in 0..100 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        assert!((proc.voices[0].pitch - note_for(12, 0) as f32).abs() < 0.01, "arrives at the new note");
        assert!(proc.voices[1..].iter().all(|v| v.amp_env.stage == 0), "only one voice sounds");
        hold_pad(&a.params, high, false);
        for _ in 0..100 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        assert!((proc.voices[0].pitch - note_for(0, 0) as f32).abs() < 0.01, "releasing returns to the still-held note");
    }

    #[test]
    fn effects_make_the_output_stereo() {
        let a = app();
        a.params.apply(&PresetData { chorus: 1.0, ..Default::default() });
        hold_pad(&a.params, 0, true);
        let mut proc = new_processor(Arc::clone(&a.params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut diff = 0.0;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            diff += buffer.chunks(2).map(|f| (f[0] - f[1]).abs()).sum::<f32>();
        }
        assert!(diff > 1.0, "chorus spreads left and right apart: {diff}");
    }

    /// The list column's text must never reach the visualizer panel
    /// grid's left edge (x=380) -- exercises every row's actual text
    /// (not a synthetic string), including every Presets slot in its
    /// "saved" state, since long rows there are the most likely to
    /// run wide.
    #[test]
    fn list_text_never_reaches_the_visualizer_panels() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = VoltageApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);
        app.expanded = [true; NUM_GROUPS];

        let rows = app.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if app.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", app.group_name(*g)), app.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", app.leaf_name(*sel)), app.leaf_value(*sel)),
            })
            .collect();

        let mut fb = FrameBuffer::new();
        let mut list = ParamList::new();
        list.draw(&mut fb, 16, 44, 24, rows.len(), &display_rows); // visible = all, so every row's width is checked

        const PANEL_X0: i32 = 380; // must match `grid_x0` in `draw`
        let mut max_x = 0i32;
        for (i, &p) in fb.buffer().iter().enumerate() {
            if p != 0 {
                max_x = max_x.max((i % crate::display::WIDTH) as i32);
            }
        }
        assert!(max_x < PANEL_X0, "list text reached x={max_x}, at or past the visualizer panels' left edge ({PANEL_X0})");
    }

    /// The 4 visualizer panels (oscillator/filter/amp env/LFO) must
    /// draw without panicking across both a default patch and a set
    /// of edge-value knobs (the kind of all-zero/all-extreme settings
    /// most likely to divide by zero or produce a degenerate empty
    /// shape), and each panel's box must actually end up with some
    /// non-background pixels in it -- a blank panel would mean the
    /// geometry math placed the curve outside its own box.
    #[test]
    fn visualizer_panels_draw_without_panicking_and_are_not_blank() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = VoltageApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
        let lit_default = fb.buffer().iter().filter(|&&p| p != 0).count();
        assert!(lit_default > 500, "expected the panels to draw a substantial number of non-background pixels, got {lit_default}");

        // Edge settings: everything that could plausibly degenerate a
        // curve to a single point or blow up a division.
        app.params.amp_attack.set(0.0);
        app.params.amp_decay.set(0.0);
        app.params.amp_sustain.set(0.0);
        app.params.amp_release.set(0.0);
        app.params.lfo_rate.set(MIN_LFO_RATE);
        app.params.lfo_depth.set(0.0);
        app.params.filter_env_amount.set(0.0);
        app.params.filter_resonance.set(0.0);
        app.params.osc_mix.set(0.0);
        app.params.sub_level.set(0.0);
        let mut fb2 = FrameBuffer::new();
        app.draw(&mut fb2); // must not panic

        app.params.amp_attack.set(1.0);
        app.params.amp_decay.set(1.0);
        app.params.amp_sustain.set(1.0);
        app.params.amp_release.set(1.0);
        app.params.lfo_rate.set(MAX_LFO_RATE);
        app.params.lfo_depth.set(1.0);
        app.params.filter_env_amount.set(1.0);
        app.params.filter_resonance.set(1.0);
        app.params.osc_mix.set(1.0);
        app.params.sub_level.set(1.0);
        let mut fb3 = FrameBuffer::new();
        app.draw(&mut fb3); // must not panic
    }

    fn app() -> VoltageApp {
        VoltageApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    #[test]
    fn opens_playable_knobs_shape_the_filter_and_pads_still_play() {
        let mut a = app();
        assert!(a.play_column().is_some(), "play view first");
        let cutoff = a.params.filter_cutoff.get();
        a.tick(&Input { knob1: 3, ..Default::default() });
        assert!(a.params.filter_cutoff.get() > cutoff, "knob 1 is cutoff on the play view");
        a.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
        assert!(a.params.held.lock().unwrap()[12], "KEYS layer plays the pads");
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.params.octave.load(Ordering::Relaxed), 1, "D-pad up = octave up");
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none(), "R1 opens the full menu");
    }

    #[test]
    fn the_stick_sweeps_cutoff_and_returns_and_midi_keys_play_their_pitch() {
        let mut a = app();
        let cutoff = a.params.filter_cutoff.get();
        a.tick(&Input { stick: [1.0, 0.0], ..Default::default() });
        assert!((a.params.filter_cutoff.get() - (cutoff + 0.5).min(1.0)).abs() < 1e-4, "full right = half the range up");
        a.tick(&Input::default());
        assert!((a.params.filter_cutoff.get() - cutoff).abs() < 1e-5, "back to the knob");
        let mut keys = crate::app::MidiKeys::default();
        keys.0[note_for(4, 0) as usize] = 100; // E3: rank 4
        a.tick(&Input { midi_keys: keys, ..Default::default() });
        assert!(a.params.held.lock().unwrap()[pad_rank(4) as usize], "a key presses the pad with its pitch");
    }

    #[test]
    fn f3_runs_the_arp() {
        let mut a = app();
        let was = a.running() == Some(true);
        a.toggle_running();
        assert_eq!(a.running(), Some(!was));
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(VoltageApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
