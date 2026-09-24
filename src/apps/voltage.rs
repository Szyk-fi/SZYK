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

use crate::app::{App, Input};
use crate::arpeggiator::{Arpeggiator, PATTERN_NAMES as ARP_PATTERN_NAMES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
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
    Osc1Wave,
    Octave,
    Osc2Wave,
    Osc2Detune,
    OscMix,
    SubLevel,
    NoiseLevel,
    Unison,
    UnisonDetune,
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
    UserPreset(usize),
    ArpOn,
    ArpPattern,
    ArpRate,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 6;
const ARP_GROUP: usize = 5;
const NUM_USER_PRESETS: usize = 8;

/// One saved snapshot of every Voltage knob -- see
/// `Selection::UserPreset`. Session-only, same as Prism's own user
/// presets: nothing in this whole sim persists to disk, so slots reset
/// when the program restarts.
#[derive(Clone, Copy, Default)]
struct UserPresetData {
    osc1_wave: u32,
    osc2_wave: u32,
    osc2_detune: f32,
    osc_mix: f32,
    sub_level: f32,
    noise_level: f32,
    unison: u32,
    unison_detune: f32,
    filter_cutoff: f32,
    filter_resonance: f32,
    filter_env_amount: f32,
    filter_attack: f32,
    filter_decay: f32,
    filter_sustain: f32,
    filter_release: f32,
    amp_attack: f32,
    amp_decay: f32,
    amp_sustain: f32,
    amp_release: f32,
    lfo_rate: f32,
    lfo_depth: f32,
    lfo_dest: u32,
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
    held: Mutex<[bool; 16]>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    /// Session-only saved knob snapshots -- see `Selection::UserPreset`
    /// and `UserPresetData`. UI-thread-only; the audio thread never
    /// touches this.
    user_presets: [Mutex<Option<UserPresetData>>; NUM_USER_PRESETS],
    /// See arpeggiator.rs -- stepped once per audio block in `process`.
    arp: Arpeggiator,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Voltage", modbus);
        Self {
            osc1_wave: AtomicU32::new(0),
            octave: AtomicI32::new(0),
            osc2_wave: AtomicU32::new(0),
            osc2_detune: AtomicF32::new(0.15),
            osc_mix: AtomicF32::new(0.5),
            sub_level: AtomicF32::new(0.3),
            noise_level: AtomicF32::new(0.0),
            unison: AtomicU32::new(1),
            unison_detune: AtomicF32::new(0.15),
            filter_cutoff: AtomicF32::new(0.6),
            ext_filter_cutoff: modbus.register("Voltage: Filter Cutoff".to_string()),
            filter_resonance: AtomicF32::new(0.3),
            // Defaults to no sweep at all -- a nonzero amount makes the
            // filter cutoff sweep with every note's envelope, which
            // (especially with resonance emphasizing the moving peak)
            // reads as a pitch "glide" rather than a plain tone. Real
            // portamento would need its own explicit oscillator-pitch
            // slew, which nothing here implements; this is the closest
            // thing to it, so it stays fully available as a knob, just
            // opt-in instead of baked into the default patch.
            filter_env_amount: AtomicF32::new(0.0),
            filter_attack: AtomicF32::new(0.0),
            filter_decay: AtomicF32::new(0.4),
            filter_sustain: AtomicF32::new(0.3),
            filter_release: AtomicF32::new(0.25),
            amp_attack: AtomicF32::new(0.02),
            amp_decay: AtomicF32::new(0.3),
            amp_sustain: AtomicF32::new(0.8),
            amp_release: AtomicF32::new(0.3),
            lfo_rate: AtomicF32::new(4.0),
            lfo_depth: AtomicF32::new(0.0),
            lfo_dest: AtomicU32::new(0),
            held: Mutex::new([false; 16]),
            bus_out: audio_bus.register("Voltage"),
            mix_level,
            ext_mix_level,
            user_presets: std::array::from_fn(|_| Mutex::new(None)),
            arp: Arpeggiator::new(),
        }
    }
}

pub struct VoltageApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
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
        Self { params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)), sensitivity, nav_speed, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![
                Selection::Osc1Wave,
                Selection::Octave,
                Selection::Osc2Wave,
                Selection::Osc2Detune,
                Selection::OscMix,
                Selection::SubLevel,
                Selection::NoiseLevel,
                Selection::Unison,
                Selection::UnisonDetune,
            ],
            1 => vec![
                Selection::FilterCutoff,
                Selection::FilterResonance,
                Selection::FilterEnvAmount,
                Selection::FilterAttack,
                Selection::FilterDecay,
                Selection::FilterSustain,
                Selection::FilterRelease,
            ],
            2 => vec![Selection::AmpAttack, Selection::AmpDecay, Selection::AmpSustain, Selection::AmpRelease],
            3 => vec![Selection::LfoRate, Selection::LfoDepth, Selection::LfoDest],
            g if g == ARP_GROUP => vec![Selection::ArpOn, Selection::ArpPattern, Selection::ArpRate],
            _ => (0..NUM_USER_PRESETS).map(Selection::UserPreset).collect(),
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
            0 => "Oscillators",
            1 => "Filter",
            2 => "Amp Envelope",
            3 => "LFO",
            g if g == ARP_GROUP => "Arp",
            _ => "Presets",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => format!(
                "{} + {}, unison x{}",
                WAVE_NAMES[self.params.osc1_wave.load(Ordering::Relaxed) as usize % 4],
                WAVE_NAMES[self.params.osc2_wave.load(Ordering::Relaxed) as usize % 4],
                self.params.unison.load(Ordering::Relaxed)
            ),
            1 => format!("cutoff {:.2}, res {:.2}", self.params.filter_cutoff.get(), self.params.filter_resonance.get()),
            2 => format!("A{:.2} D{:.2} S{:.2} R{:.2}", self.params.amp_attack.get(), self.params.amp_decay.get(), self.params.amp_sustain.get(), self.params.amp_release.get()),
            3 => LFO_DEST_NAMES[self.params.lfo_dest.load(Ordering::Relaxed) as usize % 2].to_string(),
            g if g == ARP_GROUP => self.leaf_value(Selection::ArpOn),
            _ => {
                let saved = self.params.user_presets.iter().filter(|s| s.lock().unwrap().is_some()).count();
                format!("{saved}/{NUM_USER_PRESETS} saved")
            }
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Osc1Wave => "Osc 1 Wave".into(),
            Selection::Octave => "Octave".into(),
            Selection::Osc2Wave => "Osc 2 Wave".into(),
            Selection::Osc2Detune => "Osc 2 Detune".into(),
            Selection::OscMix => "Osc Mix".into(),
            Selection::SubLevel => "Sub Level".into(),
            Selection::NoiseLevel => "Noise Level".into(),
            Selection::Unison => "Unison".into(),
            Selection::UnisonDetune => "Unison Detune".into(),
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
            Selection::UserPreset(i) => format!("Slot {}", i + 1),
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
            Selection::UserPreset(i) => {
                if self.params.user_presets[i].lock().unwrap().is_some() { "saved".into() } else { "empty".into() }
            }
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
            Selection::UserPreset(i) => {
                if step > 0 {
                    self.save_user_preset(i);
                } else {
                    self.load_user_preset(i);
                }
            }
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

    /// Saves every current knob into user preset slot `i`.
    fn save_user_preset(&self, i: usize) {
        let p = &self.params;
        let data = UserPresetData {
            osc1_wave: p.osc1_wave.load(Ordering::Relaxed),
            osc2_wave: p.osc2_wave.load(Ordering::Relaxed),
            osc2_detune: p.osc2_detune.get(),
            osc_mix: p.osc_mix.get(),
            sub_level: p.sub_level.get(),
            noise_level: p.noise_level.get(),
            unison: p.unison.load(Ordering::Relaxed),
            unison_detune: p.unison_detune.get(),
            filter_cutoff: p.filter_cutoff.get(),
            filter_resonance: p.filter_resonance.get(),
            filter_env_amount: p.filter_env_amount.get(),
            filter_attack: p.filter_attack.get(),
            filter_decay: p.filter_decay.get(),
            filter_sustain: p.filter_sustain.get(),
            filter_release: p.filter_release.get(),
            amp_attack: p.amp_attack.get(),
            amp_decay: p.amp_decay.get(),
            amp_sustain: p.amp_sustain.get(),
            amp_release: p.amp_release.get(),
            lfo_rate: p.lfo_rate.get(),
            lfo_depth: p.lfo_depth.get(),
            lfo_dest: p.lfo_dest.load(Ordering::Relaxed),
        };
        *self.params.user_presets[i].lock().unwrap() = Some(data);
    }

    /// Recalls user preset slot `i`'s knob state, if it has one saved.
    fn load_user_preset(&self, i: usize) {
        let Some(data) = *self.params.user_presets[i].lock().unwrap() else { return };
        let p = &self.params;
        p.osc1_wave.store(data.osc1_wave, Ordering::Relaxed);
        p.osc2_wave.store(data.osc2_wave, Ordering::Relaxed);
        p.osc2_detune.set(data.osc2_detune);
        p.osc_mix.set(data.osc_mix);
        p.sub_level.set(data.sub_level);
        p.noise_level.set(data.noise_level);
        p.unison.store(data.unison, Ordering::Relaxed);
        p.unison_detune.set(data.unison_detune);
        p.filter_cutoff.set(data.filter_cutoff);
        p.filter_resonance.set(data.filter_resonance);
        p.filter_env_amount.set(data.filter_env_amount);
        p.filter_attack.set(data.filter_attack);
        p.filter_decay.set(data.filter_decay);
        p.filter_sustain.set(data.filter_sustain);
        p.filter_release.set(data.filter_release);
        p.amp_attack.set(data.amp_attack);
        p.amp_decay.set(data.amp_decay);
        p.amp_sustain.set(data.amp_sustain);
        p.amp_release.set(data.amp_release);
        p.lfo_rate.set(data.lfo_rate);
        p.lfo_depth.set(data.lfo_depth);
        p.lfo_dest.store(data.lfo_dest, Ordering::Relaxed);
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
            Selection::UserPreset(i) => *self.params.user_presets[i].lock().unwrap() = None,
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
fn cutoff_hz(knob: f32) -> f32 {
    MIN_CUTOFF_HZ * (MAX_CUTOFF_HZ / MIN_CUTOFF_HZ).powf(knob.clamp(0.0, 1.0))
}

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

/// See `VoltageApp::voltage_panels`.
pub(crate) struct VoltagePanels {
    pub oscillator: Vec<f32>,
    pub filter: Vec<f32>,
    pub filter_cutoff_frac: f32,
    pub filter_swept_frac: Option<f32>,
    pub amp_env: Vec<f32>,
    pub lfo: Vec<f32>,
}

impl App for VoltageApp {
    fn supports_pad_lock(&self) -> bool { true }

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
            oscillator: seg(&p.oscillator, true),
            filter: seg(&p.filter, false),
            filter_cutoff_frac: p.filter_cutoff_frac,
            filter_swept_frac: p.filter_swept_frac,
            amp_env: seg(&p.amp_env, false),
            lfo: seg(&p.lfo, true),
        })
    }

    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
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
        Some(Box::new(VoltageProcessor { params: Arc::clone(&self.params), voices: std::array::from_fn(|i| AnalogVoice::new(i as u32)), mono_buf: Vec::new(), smoothed_headroom: 1.0 }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(VOLTAGE_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, VOLTAGE_TITLE);
        Text::new("Voltage", Point::new(16, 30), title).draw(fb).ok();

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
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, VOLTAGE_BG, VOLTAGE_DIM, VOLTAGE_ACCENT);

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
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(Selection::UserPreset(_))) => "knob2: turn one way to save, the other to load   press knob2: clear".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
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
        let q = 0.5 + (1.0 - resonance) * 9.5; // higher Q value = *less* resonant in this recurrence
        let octave = self.params.octave.load(Ordering::Relaxed);

        let mut active_voices: usize = 0;
        for i in 0..16 {
            let gate = held[i];
            let note = note_for(pad_rank(i as i32), octave) as f32;
            let base_freq = 440.0 * 2f32.powf((note - 69.0) / 12.0);
            let voice = &mut self.voices[i];
            voice.buf.clear();
            voice.buf.resize(frames, 0.0);

            for n in 0..frames {
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

                let pre_filter = osc_sum + sub + noise;

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

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
        }

        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_processor(params: Arc<Params>) -> VoltageProcessor {
        VoltageProcessor { params, voices: std::array::from_fn(|i| AnalogVoice::new(i as u32)), mono_buf: Vec::new(), smoothed_headroom: 1.0 }
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

    /// A user preset must round-trip every knob it saves, and must not
    /// clobber a slot until explicitly saved into.
    #[test]
    fn user_preset_saves_and_loads_knob_state() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = VoltageApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        app.params.osc1_wave.store(2, Ordering::Relaxed);
        app.params.osc2_wave.store(3, Ordering::Relaxed);
        app.params.osc2_detune.set(4.0);
        app.params.osc_mix.set(0.7);
        app.params.sub_level.set(0.6);
        app.params.noise_level.set(0.2);
        app.params.unison.store(3, Ordering::Relaxed);
        app.params.unison_detune.set(0.4);
        app.params.filter_cutoff.set(0.8);
        app.params.filter_resonance.set(0.5);
        app.params.filter_env_amount.set(0.9);
        app.params.filter_attack.set(0.1);
        app.params.filter_decay.set(0.2);
        app.params.filter_sustain.set(0.3);
        app.params.filter_release.set(0.4);
        app.params.amp_attack.set(0.05);
        app.params.amp_decay.set(0.6);
        app.params.amp_sustain.set(0.7);
        app.params.amp_release.set(0.8);
        app.params.lfo_rate.set(9.0);
        app.params.lfo_depth.set(0.6);
        app.params.lfo_dest.store(1, Ordering::Relaxed);

        assert!(app.params.user_presets[0].lock().unwrap().is_none(), "slot 0 should start empty");
        app.save_user_preset(0);
        assert!(app.params.user_presets[0].lock().unwrap().is_some(), "save should fill the slot");

        // Change everything, then load the slot back.
        app.params.osc1_wave.store(0, Ordering::Relaxed);
        app.params.osc2_wave.store(0, Ordering::Relaxed);
        app.params.osc2_detune.set(0.0);
        app.params.osc_mix.set(0.0);
        app.params.sub_level.set(0.0);
        app.params.noise_level.set(0.0);
        app.params.unison.store(1, Ordering::Relaxed);
        app.params.unison_detune.set(0.0);
        app.params.filter_cutoff.set(0.0);
        app.params.filter_resonance.set(0.0);
        app.params.filter_env_amount.set(0.0);
        app.params.filter_attack.set(0.0);
        app.params.filter_decay.set(0.0);
        app.params.filter_sustain.set(0.0);
        app.params.filter_release.set(0.0);
        app.params.amp_attack.set(0.0);
        app.params.amp_decay.set(0.0);
        app.params.amp_sustain.set(0.0);
        app.params.amp_release.set(0.0);
        app.params.lfo_rate.set(1.0);
        app.params.lfo_depth.set(0.0);
        app.params.lfo_dest.store(0, Ordering::Relaxed);

        app.load_user_preset(0);
        assert_eq!(app.params.osc1_wave.load(Ordering::Relaxed), 2);
        assert_eq!(app.params.osc2_wave.load(Ordering::Relaxed), 3);
        assert_eq!(app.params.osc2_detune.get(), 4.0);
        assert_eq!(app.params.osc_mix.get(), 0.7);
        assert_eq!(app.params.sub_level.get(), 0.6);
        assert_eq!(app.params.noise_level.get(), 0.2);
        assert_eq!(app.params.unison.load(Ordering::Relaxed), 3);
        assert_eq!(app.params.unison_detune.get(), 0.4);
        assert_eq!(app.params.filter_cutoff.get(), 0.8);
        assert_eq!(app.params.filter_resonance.get(), 0.5);
        assert_eq!(app.params.filter_env_amount.get(), 0.9);
        assert_eq!(app.params.filter_attack.get(), 0.1);
        assert_eq!(app.params.filter_decay.get(), 0.2);
        assert_eq!(app.params.filter_sustain.get(), 0.3);
        assert_eq!(app.params.filter_release.get(), 0.4);
        assert_eq!(app.params.amp_attack.get(), 0.05);
        assert_eq!(app.params.amp_decay.get(), 0.6);
        assert_eq!(app.params.amp_sustain.get(), 0.7);
        assert_eq!(app.params.amp_release.get(), 0.8);
        assert_eq!(app.params.lfo_rate.get(), 9.0);
        assert_eq!(app.params.lfo_depth.get(), 0.6);
        assert_eq!(app.params.lfo_dest.load(Ordering::Relaxed), 1);

        // Clearing (press on an occupied slot) empties it again.
        app.reset(Selection::UserPreset(0));
        assert!(app.params.user_presets[0].lock().unwrap().is_none(), "reset should clear the slot");
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
        for i in 0..NUM_USER_PRESETS {
            app.save_user_preset(i);
        }

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
}
