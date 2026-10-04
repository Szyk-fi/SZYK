//! Timbre Map (AI): a neural synthesizer you play by moving across a map
//! of instruments.
//!
//! The decoder of a variational autoencoder (`assets/npu/timbre.pmxn`,
//! trained by `tools/npu/train_timbre.py` on sixteen families of
//! instrument models: brass, strings, flute, clarinet, oboe, organ,
//! voices, plucked strings, piano, mallets, synths and more) turns a
//! point on a 2-D map, the note, the velocity and the time since the note
//! began into one frame of sound: the levels of 32 harmonics, 4 bands of
//! noise and a loudness. It runs every 4 ms for every sounding voice, and
//! an additive synthesizer plays what it says -- the DDSP approach, where
//! the network controls a synthesizer instead of making samples.
//!
//! The map places similar instruments near each other; the landmarks show
//! where each family landed. Between them are sounds that are neither:
//! half flute, half bowed string. Move while a note sounds and it morphs.
//!
//! Controls: pads play (C major from C3, bottom-left up), as do MIDI
//! keyboards and other apps. The D-pad or the joystick moves across the
//! map; with hands over the depth sensors, your left hand steers across
//! and your right hand up and down. SELECT jumps to the next landmark;
//! hold SELECT to let the sound drift by itself.

use crate::app::{App, Input, SlintExtra};
use crate::apps::hum::ai_header;
use crate::apps::kids_kit::{self as kit, Extra, Noise, Size2, Sound, Svf};
use crate::apps::neural::{self, Model};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const NAME: &str = "Timbre Map";
static MODEL: &[u8] = include_bytes!("../../assets/npu/timbre.pmxn");
const H: usize = 32;
const NB: usize = 4;
const OUT: usize = H + NB + 1;
const VOICES: usize = 8;
const CONTROL_S: f32 = 0.004;
const RELEASE_S: f32 = 0.3;
const NOISE_HZ: [f32; NB] = [500.0, 2000.0, 4500.0, 9000.0];
const TABLE: usize = 2048;

struct Landmark {
    name: String,
    x: f32,
    y: f32,
}

struct MapInfo {
    lo: [f32; 2],
    hi: [f32; 2],
    marks: Vec<Landmark>,
}

fn map_info(m: &Model) -> MapInfo {
    let f = |k: &str, i: usize| m.meta[k][i].as_f64().unwrap_or(0.0) as f32;
    let marks = m.meta["families"].as_array().map(|a| a.iter().map(|v| Landmark { name: v["name"].as_str().unwrap_or("").into(), x: v["x"].as_f64().unwrap_or(0.0) as f32, y: v["y"].as_f64().unwrap_or(0.0) as f32 }).collect()).unwrap_or_default();
    MapInfo { lo: [f("map_lo", 0), f("map_lo", 1)], hi: [f("map_hi", 0), f("map_hi", 1)], marks }
}

/// The decoder's inputs for one voice at one moment.
pub fn decoder_input(z: [f32; 2], midi: f32, vel: f32, t: f32) -> [f32; 5] {
    [z[0], z[1], (midi - 66.0) / 30.0, vel, (1.0 + t / 0.01).log2() / 8.0]
}

/// One decoded frame as linear amplitudes: harmonics (normalised to unit
/// power), noise bands, and the overall loudness.
pub struct Frame {
    pub harm: [f32; H],
    pub noise: [f32; NB],
    pub loud: f32,
}

pub fn to_frame(out: &[f32]) -> Frame {
    let db = |v: f32| 10f32.powf(v.clamp(-1.2, 0.5) * 60.0 / 20.0);
    let mut harm = [0.0; H];
    let mut power = 0.0;
    for k in 0..H {
        harm[k] = if out[k] < -0.95 { 0.0 } else { db(out[k]) };
        power += harm[k] * harm[k];
    }
    let n = power.sqrt().max(1e-9);
    harm.iter_mut().for_each(|a| *a /= n);
    let mut noise = [0.0; NB];
    for b in 0..NB {
        noise[b] = if out[H + b] < -0.95 { 0.0 } else { db(out[H + b]) };
    }
    Frame { harm, noise, loud: db(out[H + NB]).min(1.5) }
}

struct Voice {
    midi: f32,
    vel: f32,
    t: f32,
    held: bool,
    rel: f32,
    phase: [f32; H],
    amp: [f32; H],
    target: [f32; H],
    namp: [f32; NB],
    ntarget: [f32; NB],
    nf: [Svf; NB],
    noise: Noise,
    gain: f32,
    gain_target: f32,
}

/// The synthesizer: runs the decoder at control rate, additive voices at
/// audio rate.
struct Synth {
    model: Model,
    x: Arc<AtomicF32>,
    y: Arc<AtomicF32>,
    voices: Vec<Voice>,
    until: usize,
    out: Vec<f32>,
    sine: Vec<f32>,
    runs: Arc<AtomicU64>,
}

impl Synth {
    fn control(&mut self, sr: f32) {
        let z = [self.x.get(), self.y.get()];
        for v in self.voices.iter_mut() {
            let input = decoder_input(z, v.midi, v.vel, v.t);
            self.model.run(&input, &mut self.out);
            self.runs.fetch_add(1, Ordering::Relaxed);
            let f = to_frame(&self.out);
            let f0 = kit::midi_hz(v.midi);
            for k in 0..H {
                v.target[k] = if f0 * (k + 1) as f32 > sr * 0.45 { 0.0 } else { f.harm[k] };
            }
            v.ntarget = f.noise;
            v.gain_target = f.loud * (0.35 + 0.65 * v.vel) * 0.22;
        }
    }
}

impl Extra for Synth {
    fn event(&mut self, a: u32, b: f32, c: f32) {
        match a {
            1 => {
                if let Some(v) = self.voices.iter_mut().find(|v| v.midi == b && v.held) {
                    v.held = false;
                }
                if self.voices.len() >= VOICES {
                    // Steal the oldest.
                    let i = (0..self.voices.len()).max_by(|&i, &j| self.voices[i].t.total_cmp(&self.voices[j].t)).unwrap();
                    self.voices.remove(i);
                }
                self.voices.push(Voice { midi: b, vel: c, t: 0.0, held: true, rel: 1.0, phase: [0.0; H], amp: [0.0; H], target: [0.0; H], namp: [0.0; NB], ntarget: [0.0; NB], nf: [Svf::default(); NB], noise: Noise::new(b as u32 * 7919 + 1), gain: 0.0, gain_target: 0.0 });
                self.until = 0;
            }
            _ => {
                for v in self.voices.iter_mut().filter(|v| v.midi == b) {
                    v.held = false;
                }
            }
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        if self.voices.is_empty() {
            return (0.0, 0.0);
        }
        if self.until == 0 {
            self.control(sr);
            self.until = (CONTROL_S * sr) as usize;
        }
        self.until -= 1;
        // Glide each level to its new value over one control period.
        let k = 1.0 - (-1.0 / (CONTROL_S * sr * 0.5)).exp();
        let mut out = 0.0;
        for v in self.voices.iter_mut() {
            let f0 = kit::midi_hz(v.midi);
            let mut s = 0.0;
            for h in 0..H {
                v.amp[h] += (v.target[h] - v.amp[h]) * k;
                if v.amp[h] < 1e-5 {
                    continue;
                }
                let p = v.phase[h];
                let idx = p * TABLE as f32;
                let i = idx as usize;
                let fr = idx - i as f32;
                s += v.amp[h] * (self.sine[i] * (1.0 - fr) + self.sine[i + 1] * fr);
                v.phase[h] = (p + f0 * (h + 1) as f32 / sr).fract();
            }
            let w = v.noise.next();
            for b in 0..NB {
                v.namp[b] += (v.ntarget[b] - v.namp[b]) * k;
                if v.namp[b] > 1e-4 {
                    s += v.nf[b].tick(w, NOISE_HZ[b], 1.0, sr).bp * v.namp[b] * 0.5;
                }
            }
            v.gain += (v.gain_target - v.gain) * k;
            if !v.held {
                v.rel *= (-1.0 / (RELEASE_S * sr / 4.6)).exp();
            }
            out += s * v.gain * v.rel;
            v.t += 1.0 / sr;
        }
        self.voices.retain(|v| v.rel > 1e-4 && v.t < 30.0);
        (out, out)
    }
}

pub struct TimbreMap {
    sound: Sound,
    info: MapInfo,
    x: Arc<AtomicF32>,
    y: Arc<AtomicF32>,
    preview: Model,
    spectrum: [f32; H],
    noise_view: [f32; NB],
    trail: Vec<(f32, f32)>,
    prev_pads: [bool; 16],
    prev_keys: [u8; 128],
    mark: usize,
    drift: bool,
    drift_t: f32,
    runs: Arc<AtomicU64>,
    rate: f64,
    counted: (u64, std::time::Instant),
}

impl TimbreMap {
    pub fn new(sound: Sound) -> TimbreMap {
        let preview = Model::from_bytes(MODEL).expect("timbre model");
        let info = map_info(&preview);
        sound.set_reverb(0.25);
        let start = info.marks.first().map_or((0.0, 0.0), |m| (m.x, m.y));
        TimbreMap { sound, info, x: Arc::new(AtomicF32::new(start.0)), y: Arc::new(AtomicF32::new(start.1)), preview, spectrum: [0.0; H], noise_view: [0.0; NB], trail: Vec::new(), prev_pads: [false; 16], prev_keys: [0; 128], mark: 0, drift: false, drift_t: 0.0, runs: Arc::new(AtomicU64::new(0)), rate: 0.0, counted: (0, std::time::Instant::now()) }
    }

    fn nearest(&self) -> &str {
        let (x, y) = (self.x.get(), self.y.get());
        self.info.marks.iter().min_by(|a, b| ((a.x - x).powi(2) + (a.y - y).powi(2)).total_cmp(&((b.x - x).powi(2) + (b.y - y).powi(2)))).map_or("", |m| m.name.as_str())
    }

    fn set(&mut self, x: f32, y: f32) {
        let (lo, hi) = (self.info.lo, self.info.hi);
        let pad = [(hi[0] - lo[0]) * 0.1, (hi[1] - lo[1]) * 0.1];
        self.x.set(x.clamp(lo[0] - pad[0], hi[0] + pad[0]));
        self.y.set(y.clamp(lo[1] - pad[1], hi[1] + pad[1]));
    }
}

/// The pad's note: C major from C3, bottom-left up.
fn pad_note(p: usize) -> f32 {
    let rank = (3 - p / 4) * 4 + p % 4;
    kit::scale_note(48, &kit::MAJOR, rank as i32) as f32
}

impl App for TimbreMap {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("Nearest".into(), self.nearest().into(), false), ("Map".into(), format!("{:.2}, {:.2}", self.x.get(), self.y.get()), false)]
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        std::array::from_fn(|p| if (pad_note(p) as i32) % 12 == 0 { PadColor::Blue } else { PadColor::Off })
    }

    fn tick(&mut self, input: &Input) {
        for p in 0..16 {
            if input.grid[p] != self.prev_pads[p] {
                let n = pad_note(p);
                if input.grid[p] {
                    self.sound.custom(1, n, 0.8);
                } else {
                    self.sound.custom(0, n, 0.0);
                }
            }
        }
        self.prev_pads = input.grid;
        for n in 0..128 {
            let v = input.midi_keys.0[n];
            if v != self.prev_keys[n] {
                if v > 0 && self.prev_keys[n] == 0 {
                    self.sound.custom(1, n as f32, v as f32 / 127.0);
                } else if v == 0 {
                    self.sound.custom(0, n as f32, 0.0);
                }
            }
        }
        self.prev_keys = input.midi_keys.0;
        // Moving across the map.
        let span = [self.info.hi[0] - self.info.lo[0], self.info.hi[1] - self.info.lo[1]];
        let (mut x, mut y) = (self.x.get(), self.y.get());
        x += input.knob2.signum() as f32 * span[0] * 0.03 + input.stick[0] * span[0] * 0.012;
        y -= input.navigation_steps.signum() as f32 * span[1] * 0.03;
        y += input.stick[1] * span[1] * 0.012;
        if input.hands[0] > 0.05 || input.hands[1] > 0.05 {
            let tx = self.info.lo[0] + input.hands[0].clamp(0.0, 1.0) * span[0];
            let ty = self.info.lo[1] + input.hands[1].clamp(0.0, 1.0) * span[1];
            x += (tx - x) * 0.15;
            y += (ty - y) * 0.15;
        }
        if input.knob1_press && !self.info.marks.is_empty() {
            self.mark = (self.mark + 1) % self.info.marks.len();
            x = self.info.marks[self.mark].x;
            y = self.info.marks[self.mark].y;
        }
        if input.knob2_press {
            self.drift = !self.drift;
        }
        if self.drift {
            // A slow Lissajous wander through the middle of the map.
            self.drift_t += 1.0 / 60.0;
            let cx = (self.info.lo[0] + self.info.hi[0]) / 2.0;
            let cy = (self.info.lo[1] + self.info.hi[1]) / 2.0;
            x = cx + span[0] * 0.4 * (self.drift_t * 0.13).sin();
            y = cy + span[1] * 0.4 * (self.drift_t * 0.09 + 1.0).sin();
        }
        self.set(x, y);
        if self.trail.last() != Some(&(self.x.get(), self.y.get())) {
            self.trail.push((self.x.get(), self.y.get()));
            if self.trail.len() > 120 {
                self.trail.remove(0);
            }
        }
        // What the decoder makes here, for the picture.
        let mut out = [0.0; OUT];
        self.preview.run(&decoder_input([self.x.get(), self.y.get()], 60.0, 0.8, 0.3), &mut out);
        let f = to_frame(&out);
        self.spectrum = f.harm;
        self.noise_view = f.noise;
        let e = self.counted.1.elapsed().as_secs_f64();
        if e >= 1.0 {
            let n = self.runs.load(Ordering::Relaxed);
            self.rate = (n - self.counted.0) as f64 / e;
            self.counted = (n, std::time::Instant::now());
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(2, 3, 6);
        let panel = Rgb565::new(5, 7, 13);
        let dim = Rgb565::new(14, 22, 24);
        let ink = kit::rgb(250, 200, 120);
        kit::clear(fb, bg);
        ai_header(fb, NAME, panel);
        // The map.
        let (mx, my, mw, mh) = (8, 36, 380, 296);
        kit::round_rect(fb, mx, my, mw, mh, 10, panel);
        let (lo, hi) = (self.info.lo, self.info.hi);
        let to_px = |x: f32, y: f32| (mx + 10 + ((x - lo[0]) / (hi[0] - lo[0]) * (mw - 20) as f32) as i32, my + mh - 10 - ((y - lo[1]) / (hi[1] - lo[1]) * (mh - 20) as f32) as i32);
        for (i, m) in self.info.marks.iter().enumerate() {
            let (px, py) = to_px(m.x, m.y);
            kit::circle(fb, px, py, 5, kit::rainbow(i));
            kit::text(fb, &m.name, px + 8, py - 6, Size2::Small, dim, -1);
        }
        for (i, &(x, y)) in self.trail.iter().enumerate() {
            let (px, py) = to_px(x, y);
            kit::rect(fb, px, py, 2, 2, kit::blend(panel, ink, i as f32 / self.trail.len() as f32));
        }
        let (cx, cy) = to_px(self.x.get(), self.y.get());
        kit::ring(fb, cx, cy, 10, 2, kit::WHITE);
        kit::circle(fb, cx, cy, 3, ink);
        // The sound here: its harmonics and noise.
        let (sx, sy, sw, sh) = (396, 36, 236, 170);
        kit::round_rect(fb, sx, sy, sw, sh, 10, panel);
        kit::text(fb, &format!("near {}", self.nearest()), sx + 8, sy + 6, Size2::Small, kit::WHITE, -1);
        let maxa = self.spectrum.iter().cloned().fold(1e-9f32, f32::max);
        for k in 0..H {
            let db = 20.0 * (self.spectrum[k] / maxa).max(1e-4).log10();
            let h = ((db + 60.0) / 60.0 * 120.0).max(0.0) as i32;
            kit::rect(fb, sx + 8 + k as i32 * 6, sy + sh - 12 - h, 4, h, ink);
        }
        for b in 0..NB {
            let db = 20.0 * self.noise_view[b].max(1e-4).log10();
            let h = ((db + 60.0) / 60.0 * 120.0).max(0.0) as i32;
            kit::rect(fb, sx + 206 + b as i32 * 6, sy + sh - 12 - h, 4, h, kit::rgb(120, 160, 250));
        }
        kit::text(fb, "harmonics 1-32          noise", sx + 8, sy + sh - 11, Size2::Small, dim, -1);
        kit::paragraph(fb, "Pads play. D-pad, stick or hands move across the map. SELECT: next landmark. Hold SELECT: drift.", sx, 214, sw, Size2::Small, dim);
        if self.drift {
            kit::text(fb, "drifting", sx + 8, 300, Size2::Small, ink, -1);
        }
        let macs = self.preview.macs();
        kit::footer(fb, &format!("NPU  {:.0}/s x {:.0}k MAC = {:.1} M MAC/s  NPU ~{:.2}%", self.rate, macs as f64 / 1000.0, self.rate * macs as f64 / 1e6, (self.rate * neural::npu_micros(macs) * 1e-4).abs()), panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let sine: Vec<f32> = (0..=TABLE).map(|i| (i as f32 / TABLE as f32 * std::f32::consts::TAU).sin()).collect();
        let synth = Synth { model: Model::from_bytes(MODEL).expect("timbre model"), x: Arc::clone(&self.x), y: Arc::clone(&self.y), voices: Vec::with_capacity(VOICES), until: 0, out: vec![0.0; OUT], sine, runs: Arc::clone(&self.runs) };
        Some(self.sound.processor(None, Some(Box::new(synth))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(TimbreMap::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, pitch_hz, render};

    #[test]
    fn the_model_matches_python_bit_for_bit() {
        crate::apps::neural::tests::check_vectors(MODEL, include_str!("../../assets/npu/timbre.test.json"));
    }

    fn frame_at(m: &mut Model, name: &str, info: &MapInfo, midi: f32, t: f32) -> Frame {
        let l = info.marks.iter().find(|l| l.name == name).unwrap();
        let mut out = [0.0; OUT];
        m.run(&decoder_input([l.x, l.y], midi, 0.8, t), &mut out);
        to_frame(&out)
    }

    #[test]
    fn the_landmarks_sound_like_their_families() {
        let mut m = Model::from_bytes(MODEL).unwrap();
        let info = map_info(&m);
        assert_eq!(info.marks.len(), 16);
        // A clarinet's even harmonics are much weaker than its odd ones.
        let c = frame_at(&mut m, "Clarinet", &info, 60.0, 0.3);
        let odd: f32 = (0..8).step_by(2).map(|k| c.harm[k] * c.harm[k]).sum();
        let even: f32 = (1..8).step_by(2).map(|k| c.harm[k] * c.harm[k]).sum();
        assert!(odd > even * 4.0, "clarinet odd {odd} even {even}");
        // A saw synth is brighter than a sine pad.
        let bright = |f: &Frame| (2..H).map(|k| f.harm[k] * f.harm[k]).sum::<f32>();
        let saw = frame_at(&mut m, "Saw synth", &info, 60.0, 0.3);
        let pad = frame_at(&mut m, "Sine pad", &info, 60.0, 0.3);
        assert!(bright(&saw) > bright(&pad) * 5.0, "saw {} pad {}", bright(&saw), bright(&pad));
        // A plucked string dies away; an organ doesn't.
        let p0 = frame_at(&mut m, "Plucked", &info, 60.0, 0.05).loud;
        let p1 = frame_at(&mut m, "Plucked", &info, 60.0, 3.0).loud;
        let o0 = frame_at(&mut m, "Organ", &info, 60.0, 0.05).loud;
        let o1 = frame_at(&mut m, "Organ", &info, 60.0, 3.0).loud;
        assert!(p1 < p0 * 0.3 && o1 > o0 * 0.6, "pluck {p0}->{p1} organ {o0}->{o1}");
    }

    #[test]
    fn a_pad_plays_in_tune_and_lets_go() {
        let mut app = TimbreMap::new(Sound::detached());
        app.sound.set_reverb(0.0);
        let mut p = app.audio_processor().unwrap();
        // The organ landmark: steady.
        let l = app.info.marks.iter().find(|l| l.name == "Organ").unwrap();
        app.x.set(l.x);
        app.y.set(l.y);
        app.tick(&Input { grid: std::array::from_fn(|g| g == 12), ..Default::default() });
        let out = render(&mut p, 120);
        let left: Vec<f32> = out.iter().step_by(2).copied().skip(12_000).collect();
        let hz = pitch_hz(&left, 48_000.0);
        let r = hz / 130.81;
        assert!((r - r.round()).abs() < 0.02 && r.round() >= 1.0, "C3's harmonics: {hz}");
        assert!(energy(&out) > 1e-5);
        app.tick(&Input::default());
        render(&mut p, 60);
        assert!(energy(&render(&mut p, 10)) < 1e-8, "released");
        assert!(app.runs.load(Ordering::Relaxed) > 100, "the decoder ran every 4 ms");
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
