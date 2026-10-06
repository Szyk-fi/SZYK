//! One Hydra voice: three oscillators (analog, wavetable, FM, pluck or
//! noise, with unison), a sub oscillator, ring modulation, cross modulation
//! and hard sync, two filters, three envelopes, two LFOs and the modulation
//! matrix. The engine (engine.rs) owns sixteen of these.
//!
//! Parameters change every `CTRL` samples (the control block); anything that
//! would click if it stepped is smoothed per sample.

use super::dsp::*;
use super::params::*;
use super::store::Snapshot;
use super::tables::{Tables, BANKS};
use std::f32::consts::{FRAC_PI_4, TAU};

pub const MAX_UNI: usize = 7;
/// Samples per control block.
pub const CTRL: usize = 16;
/// Longest pluck / comb delay (samples): a 10 Hz note at 192 kHz.
const LINE: usize = 20_480;

/// FM modulator-to-carrier ratios, picked by P1.
const RATIOS: [f32; 16] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0];

const OSC_TYPE_ANALOG: usize = 0;
const OSC_TYPE_WAVETABLE: usize = 1;
const OSC_TYPE_FM: usize = 2;
const OSC_TYPE_PLUCK: usize = 3;

/// Where each oscillator's parameters start in a snapshot (Type, Level,
/// Coarse, Fine, P1, P2, P3, Pan, in that order).
const OSC_BASE: [usize; 3] = [P::A_Type as usize, P::B_Type as usize, P::C_Type as usize];

fn s(snap: &Snapshot, p: P) -> f32 {
    snap[p as usize]
}

/// The note being played, and how.
#[derive(Clone, Copy)]
pub struct NoteStart {
    pub note: f32,
    pub velocity: f32,
    /// Keep the phases and the pitch running (legato) instead of restarting.
    pub legato: bool,
    /// Restart the envelopes even when legato (a mono synth's new note).
    pub retrigger: bool,
}

/// Karplus-Strong string state for one oscillator.
struct Pluck {
    line: Vec<f32>,
    pos: usize,
    last: f32,
    noise_lp: f32,
}

impl Pluck {
    fn new() -> Self {
        Self { line: vec![0.0; LINE], pos: 0, last: 0.0, noise_lp: 0.0 }
    }

    /// Fills the string with a burst of noise as long as one period.
    fn excite(&mut self, delay: f32, bright: f32, rng: &mut Rng) {
        let n = (delay as usize).clamp(2, LINE - 2);
        let a = 0.1 + 0.9 * bright;
        self.noise_lp = 0.0;
        for i in 0..n {
            self.noise_lp += (rng.bipolar() - self.noise_lp) * a;
            self.line[(self.pos + i) % LINE] = self.noise_lp * (1.0 + 2.0 * (1.0 - a));
        }
        for i in n..LINE.min(n + 64) {
            self.line[(self.pos + i) % LINE] = 0.0;
        }
        self.last = 0.0;
    }

    #[inline]
    fn tick(&mut self, delay: f32, damping: f32, decay: f32) -> f32 {
        let d = delay.clamp(2.0, LINE as f32 - 2.0);
        let rp = self.pos as f32 + LINE as f32 - d;
        let i0 = rp as usize % LINE;
        let f = rp.fract();
        let a = self.line[i0];
        let b = self.line[(i0 + 1) % LINE];
        let y = a + (b - a) * f;
        let lp = 0.5 * (y + self.last);
        self.last = y;
        let v = (y + (lp - y) * (0.15 + 0.85 * damping)) * decay;
        self.line[self.pos] = v.clamp(-4.0, 4.0);
        self.pos = (self.pos + 1) % LINE;
        y
    }
}

/// One oscillator's running state (unison phases, FM operators, string).
struct Osc {
    phase: [f32; MAX_UNI],
    fm_c: f32,
    fm_m: f32,
    fm_prev: f32,
    pluck: Pluck,
    noise_lp: f32,
    noise_bp: Svf,
}

impl Osc {
    fn new() -> Self {
        Self { phase: [0.0; MAX_UNI], fm_c: 0.0, fm_m: 0.0, fm_prev: 0.0, pluck: Pluck::new(), noise_lp: 0.0, noise_bp: Svf::default() }
    }
}

/// An oscillator's parameters for the current control block, after the
/// modulation matrix.
#[derive(Clone, Copy, Default)]
struct OscCtl {
    kind: usize,
    semis: f32,
    level: f32,
    p1: f32,
    p2: f32,
    p3: f32,
    pan: f32,
}

/// Everything the control block works out.
#[derive(Clone, Copy, Default)]
struct Ctl {
    osc: [OscCtl; 3],
    pitch: f32,
    sub: f32,
    ring: f32,
    noise: f32,
    cut1: f32,
    res1: f32,
    drv1: f32,
    cut2: f32,
    res2: f32,
    drv2: f32,
    amp: f32,
    pan: f32,
    uni_detune: f32,
    xmod: f32,
    stereo: bool,
}

pub struct Voice {
    pub active: bool,
    pub gate: bool,
    /// The note that started it, for note-off matching.
    pub key: u8,
    pub age: u64,
    pub velocity: f32,
    note: f32,
    note_cur: f32,
    /// A note waiting for this voice's fast fade-out (voice stealing).
    pub pending: Option<NoteStart>,
    pub pending_key: u8,
    fast: bool,
    osc: [Osc; 3],
    sub_phase: f32,
    /// The mixer's own noise source colour filter.
    mixer_noise: f32,
    f1: [(Svf, Svf); 2],
    f2: [Ladder; 2],
    comb: [Comb; 2],
    amp_env: Env,
    env2: Env,
    env3: Env,
    lfo1: Lfo,
    lfo2: Lfo,
    held_for: f32,
    random: f32,
    drift_target: f32,
    drift: OnePole,
    drift_count: u32,
    cut1: OnePole,
    cut2: OnePole,
    gain: OnePole,
    rng: Rng,
    /// Level of the amp envelope as of the last block, for the UI and stealing.
    pub level: f32,
}

impl Voice {
    pub fn new(seed: u32) -> Self {
        Self {
            active: false,
            gate: false,
            key: 0,
            age: 0,
            velocity: 0.0,
            note: 60.0,
            note_cur: 60.0,
            pending: None,
            pending_key: 0,
            fast: false,
            osc: [Osc::new(), Osc::new(), Osc::new()],
            sub_phase: 0.0,
            mixer_noise: 0.0,
            f1: [(Svf::default(), Svf::default()); 2],
            f2: [Ladder::default(); 2],
            comb: [Comb::new(LINE), Comb::new(LINE)],
            amp_env: Env::default(),
            env2: Env::default(),
            env3: Env::default(),
            lfo1: Lfo::new(seed ^ 0x1111),
            lfo2: Lfo::new(seed ^ 0x2222),
            held_for: 0.0,
            random: 0.0,
            drift_target: 0.0,
            drift: OnePole::new(0.0),
            drift_count: 0,
            cut1: OnePole::new(8000.0),
            cut2: OnePole::new(8000.0),
            gain: OnePole::new(0.0),
            rng: Rng::new(seed),
            level: 0.0,
        }
    }

    pub fn releasing(&self) -> bool {
        self.active && !self.gate
    }

    /// Starts (or, legato, re-pitches) the voice on a note.
    pub fn note_on(&mut self, n: NoteStart, key: u8, age: u64, snap: &Snapshot, rate: f32) {
        self.key = key;
        self.age = age;
        self.velocity = n.velocity;
        self.note = n.note;
        self.gate = true;
        self.fast = false;
        self.pending = None;
        let was_active = self.active;
        self.active = true;
        if n.legato && was_active {
            self.amp_env.gate_on();
            if n.retrigger {
                self.env2.gate_on();
                self.env3.gate_on();
            }
            return;
        }
        self.note_cur = n.note;
        self.random = self.rng.bipolar();
        self.held_for = 0.0;
        self.amp_env.gate_on();
        self.env2.gate_on();
        self.env3.gate_on();
        let key_sync = s(snap, P::KeySync) >= 0.5;
        for (o, osc) in self.osc.iter_mut().enumerate() {
            let base = OSC_BASE[o];
            for (i, p) in osc.phase.iter_mut().enumerate() {
                *p = if key_sync { 0.0 } else { self.rng.unit() * (i > 0) as u8 as f32 };
            }
            osc.fm_c = 0.0;
            osc.fm_m = 0.0;
            osc.fm_prev = 0.0;
            if snap[base] as usize == OSC_TYPE_PLUCK {
                let hz = midi_hz(n.note + snap[base + 2] + snap[base + 3] / 100.0 + s(snap, P::Octave) * 12.0);
                osc.pluck.excite(rate / hz.max(10.0), snap[base + 5], &mut self.rng);
            }
        }
        self.sub_phase = 0.0;
        let l1 = if s(snap, P::L1_Mode) >= 0.5 { s(snap, P::L1_Phase) } else { self.lfo1.phase };
        let l2 = if s(snap, P::L2_Mode) >= 0.5 { s(snap, P::L2_Phase) } else { self.lfo2.phase };
        self.lfo1.retrigger(l1);
        self.lfo2.retrigger(l2);
    }

    pub fn note_off(&mut self) {
        self.gate = false;
        self.amp_env.gate_off();
        self.env2.gate_off();
        self.env3.gate_off();
    }

    /// Fades the voice out in a few milliseconds so another note can take it.
    pub fn steal_for(&mut self, n: NoteStart, key: u8) {
        self.pending = Some(n);
        self.pending_key = key;
        self.fast = true;
        self.gate = false;
        self.amp_env.gate_off();
    }

    pub fn kill(&mut self) {
        self.active = false;
        self.gate = false;
        self.pending = None;
        self.amp_env.kill();
        self.env2.kill();
        self.env3.kill();
        for f in self.f1.iter_mut() {
            f.0.reset();
            f.1.reset();
        }
        for f in self.f2.iter_mut() {
            f.reset();
        }
        for c in self.comb.iter_mut() {
            c.reset();
        }
        self.cut1 = OnePole::new(self.cut1.y);
        self.gain = OnePole::new(0.0);
        self.level = 0.0;
    }

    /// Re-pitch a held voice (mono glide, arpeggio tie).
    pub fn set_note(&mut self, note: f32) {
        self.note = note;
    }

    /// The pitch the voice is on, for the display.
    pub fn pitch(&self) -> f32 {
        self.note_cur
    }

    /// Works out this block's parameters: runs the LFOs and the mod matrix.
    fn control(&mut self, snap: &Snapshot, rate: f32, dt: f32) -> Ctl {
        let key = ((self.note_cur - 60.0) / 60.0).clamp(-1.0, 1.0);
        let mut m = [0.0f32; TARGETS.len()];
        // LFO rate modulation first (it feeds back into the LFOs below)
        let src = |idx: usize, me: &Self| -> f32 {
            match idx {
                SRC_AMP_ENV => me.amp_env.level,
                SRC_ENV2 => me.env2.level,
                SRC_ENV3 => me.env3.level,
                SRC_VELOCITY => me.velocity,
                SRC_KEY => key,
                SRC_MOD_WHEEL => s(snap, P::ModWheel),
                SRC_RANDOM => me.random,
                i if i >= SRC_MACRO1 => snap[P::Mac1 as usize + (i - SRC_MACRO1)],
                _ => 0.0,
            }
        };
        // LFOs run on this block's rate modulation from the previous values
        let fade = |len: f32, held: f32| if len <= 0.001 { 1.0 } else { (held / len).min(1.0) };
        let mut lfo_rate_mod = [0.0f32; 2];
        for slot in 0..12 {
            let base = P::M1_Src as usize + slot * 3;
            let (sr, dst, amt) = (snap[base] as usize, snap[base + 1] as usize, snap[base + 2]);
            if dst == T_LFO1_RATE || dst == T_LFO2_RATE {
                let v = match sr {
                    SRC_LFO1 | SRC_LFO2 => 0.0,
                    other => src(other, self),
                };
                lfo_rate_mod[(dst == T_LFO2_RATE) as usize] += v * amt * 3.0;
            }
        }
        let l1 = self.lfo1.tick(s(snap, P::L1_Shape) as usize, s(snap, P::L1_Rate) * lfo_rate_mod[0].exp2(), CTRL as f32 * dt) * fade(s(snap, P::L1_Fade), self.held_for);
        let l2 = self.lfo2.tick(s(snap, P::L2_Shape) as usize, s(snap, P::L2_Rate) * lfo_rate_mod[1].exp2(), CTRL as f32 * dt) * fade(s(snap, P::L2_Fade), self.held_for);
        for slot in 0..12 {
            let base = P::M1_Src as usize + slot * 3;
            let (sr, dst, amt) = (snap[base] as usize, snap[base + 1] as usize, snap[base + 2]);
            if sr == SRC_OFF || dst == T_OFF || amt == 0.0 {
                continue;
            }
            let v = match sr {
                SRC_LFO1 => l1,
                SRC_LFO2 => l2,
                other => src(other, self),
            };
            m[dst] += v * amt;
        }

        let mut c = Ctl::default();
        let global_pitch = s(snap, P::Octave) * 12.0 + s(snap, P::Tune) / 100.0 + m[T_PITCH] * 24.0 + self.drift.y;
        c.pitch = global_pitch;
        let mut pans = 0.0f32;
        for o in 0..3 {
            let b = OSC_BASE[o];
            let kind = snap[b] as usize;
            c.osc[o] = OscCtl {
                kind,
                semis: snap[b + 2] + snap[b + 3] / 100.0 + m[T_PITCH_A + o] * 24.0,
                level: (snap[b + 1] * (1.0 + m[T_A_LEVEL + o])).clamp(0.0, 2.0),
                p1: (snap[b + 4] + m[T_A_P1 + o * 3]).clamp(0.0, 1.0),
                p2: (snap[b + 5] + m[T_A_P1 + o * 3 + 1]).clamp(0.0, 1.0),
                p3: (snap[b + 6] + m[T_A_P1 + o * 3 + 2]).clamp(0.0, 1.0),
                pan: (snap[b + 7] + m[T_PAN]).clamp(-1.0, 1.0),
            };
            pans += c.osc[o].pan.abs() * (c.osc[o].level > 0.0) as u8 as f32;
        }
        c.sub = (s(snap, P::SubLevel) + m[T_SUB]).clamp(0.0, 1.0);
        c.ring = (s(snap, P::Ring) + m[T_RING]).clamp(0.0, 1.0);
        c.noise = (s(snap, P::Noise) + m[T_NOISE]).clamp(0.0, 1.0);
        let env2 = self.env2.level;
        let nyq = 0.45 * rate;
        c.cut1 = (s(snap, P::F1_Cut) * (env2 * s(snap, P::F1_Env) * 6.0 + m[T_F1_CUT] * 5.0 + (self.note_cur - 60.0) / 12.0 * s(snap, P::F1_Key)).exp2()).clamp(16.0, nyq.min(22_000.0));
        c.res1 = (s(snap, P::F1_Res) + m[T_F1_RES]).clamp(0.0, 1.0);
        c.drv1 = (s(snap, P::F1_Drive) + m[T_F1_DRIVE]).clamp(0.0, 1.0);
        c.cut2 = (s(snap, P::F2_Cut) * (env2 * s(snap, P::F2_Env) * 6.0 + m[T_F2_CUT] * 5.0 + (self.note_cur - 60.0) / 12.0 * s(snap, P::F2_Key)).exp2()).clamp(16.0, nyq.min(22_000.0));
        c.res2 = (s(snap, P::F2_Res) + m[T_F2_RES]).clamp(0.0, 1.0);
        c.drv2 = (s(snap, P::F2_Drive) + m[T_F2_DRIVE]).clamp(0.0, 1.0);
        c.amp = (1.0 + m[T_AMP]).clamp(0.0, 2.0);
        c.pan = m[T_PAN].clamp(-1.0, 1.0);
        c.uni_detune = (s(snap, P::UniDetune) + m[T_UNI_DETUNE]).clamp(0.0, 1.0);
        c.xmod = (s(snap, P::XMod) + m[T_XMOD]).clamp(0.0, 1.0);
        c.stereo = (s(snap, P::Unison) > 1.5 && s(snap, P::UniSpread) > 0.01) || pans > 0.001 || c.pan.abs() > 0.001;
        c
    }

    /// Renders `frames` samples, adding them into `out_l` / `out_r`.
    /// Returns whether the voice is still sounding.
    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32], snap: &Snapshot, rate: f32, tables: &Tables) -> bool {
        let frames = out_l.len();
        let dt = 1.0 / rate;
        let uni = (s(snap, P::Unison) as usize).clamp(1, MAX_UNI);
        let vel_sens = s(snap, P::VelSens);
        let glide = s(snap, P::Glide);
        let glide_coef = if glide <= 0.0015 { 1.0 } else { 1.0 - (-1.0 / (glide * rate)).exp() };
        let cut_coef = smooth_coef(0.004, rate);
        let gain_coef = smooth_coef(0.002, rate);
        let (att, dec, sus, rel) = (s(snap, P::AmpA), s(snap, P::AmpD), s(snap, P::AmpS), s(snap, P::AmpR));
        let (e2a, e2d, e2s, e2r) = (s(snap, P::E2A), s(snap, P::E2D), s(snap, P::E2S), s(snap, P::E2R));
        let (e3a, e3d, e3s, e3r) = (s(snap, P::E3A), s(snap, P::E3D), s(snap, P::E3S), s(snap, P::E3R));
        let f1_type = s(snap, P::F1_Type) as usize;
        let f2_type = s(snap, P::F2_Type) as usize;
        let parallel = s(snap, P::Route) >= 0.5;
        let fmix = s(snap, P::FMix);
        let noise_color = s(snap, P::NoiseColor);
        let sub_square = s(snap, P::SubShape) >= 0.5;
        let sub_div = if s(snap, P::SubOct) >= 0.5 { 0.25 } else { 0.5 };
        let b_sync = s(snap, P::B_Sync) >= 0.5;
        let drift_amt = s(snap, P::Drift);
        let vel_gain = (1.0 - vel_sens) + vel_sens * self.velocity;

        let mut i = 0;
        while i < frames {
            let n = (frames - i).min(CTRL);
            // a stolen voice that has faded out starts its waiting note
            if self.pending.is_some() && self.amp_env.idle() {
                if let Some(p) = self.pending.take() {
                    let key = self.pending_key;
                    let age = self.age;
                    self.note_on(p, key, age, snap, rate);
                }
            }
            if !self.active {
                return false;
            }
            // analog drift: a slow random walk
            if self.drift_count == 0 {
                self.drift_target = self.rng.bipolar() * 0.12 * drift_amt;
                self.drift_count = (0.25 * rate / CTRL as f32) as u32 + 1;
            }
            self.drift_count -= 1;
            self.drift.tick(self.drift_target, smooth_coef(0.5, rate / CTRL as f32));
            self.held_for += n as f32 * dt;
            let c = self.control(snap, rate, dt);
            let mut base_hz = [0.0f32; 3];
            let mut level_idx = [0usize; 3];
            for o in 0..3 {
                level_idx[o] = Tables::level_for(midi_hz(self.note_cur + c.pitch + c.osc[o].semis + 0.8) * (1.0 + c.uni_detune * 0.06), rate);
            }
            for k in 0..n {
                // glide toward the target note
                self.note_cur += (self.note - self.note_cur) * glide_coef;
                let ae = self.amp_env.tick(att, dec, sus, if self.fast { 0.003 } else { rel }, rate);
                self.env2.tick(e2a, e2d, e2s, e2r, rate);
                self.env3.tick(e3a, e3d, e3s, e3r, rate);
                let pitch = self.note_cur + c.pitch;
                for o in 0..3 {
                    base_hz[o] = midi_hz(pitch + c.osc[o].semis);
                }
                // ---- oscillators
                let (mut mix_l, mut mix_r) = (0.0f32, 0.0f32);
                let mut a_mono = 0.0f32;
                let mut b_mono = 0.0f32;
                let mut a_wrapped = false;
                for o in 0..3 {
                    let oc = c.osc[o];
                    if oc.level <= 0.0 && !(o < 2 && c.ring > 0.0) && !(o == 0 && c.xmod > 0.0) {
                        // still advance phases so the oscillator stays in step
                        let hz = base_hz[o];
                        let dtp = hz * dt;
                        let osc = &mut self.osc[o];
                        for p in osc.phase.iter_mut().take(uni) {
                            *p += dtp;
                            *p -= p.floor();
                        }
                        continue;
                    }
                    let hz = base_hz[o];
                    let dtp = (hz * dt).min(0.45);
                    let pm = if o == 1 { c.xmod * a_mono * 0.25 } else { 0.0 };
                    let (mut mono, mut l, mut r) = (0.0f32, 0.0f32, 0.0f32);
                    match oc.kind {
                        OSC_TYPE_ANALOG | OSC_TYPE_WAVETABLE => {
                            let norm = 1.0 / (uni as f32).sqrt();
                            for u in 0..uni {
                                let pos = if uni > 1 { 2.0 * u as f32 / (uni - 1) as f32 - 1.0 } else { 0.0 };
                                let det = (pos * c.uni_detune * 60.0 / 1200.0).exp2();
                                let inc = dtp * det;
                                let osc = &mut self.osc[o];
                                let ph = &mut osc.phase[u];
                                *ph += inc;
                                if *ph >= 1.0 {
                                    *ph -= ph.floor();
                                    if o == 0 && u == 0 {
                                        a_wrapped = true;
                                    }
                                }
                                let p = (*ph + pm).rem_euclid(1.0);
                                let y = if oc.kind == OSC_TYPE_ANALOG {
                                    let wpos = oc.p1 * 3.0;
                                    let wi = (wpos as usize).min(2);
                                    let wt = wpos - wi as f32;
                                    let wave = |k: usize| match k {
                                        0 => (p * TAU).sin(),
                                        1 => triangle(p, inc),
                                        2 => saw(p, inc),
                                        _ => pulse(p, inc, oc.p2),
                                    };
                                    let y = wave(wi) * (1.0 - wt) + wave(wi + 1) * wt;
                                    fold(y, oc.p3)
                                } else {
                                    let bank = (oc.p2 * (BANKS - 1) as f32).round() as usize;
                                    let bend = oc.p3 * 8.0;
                                    let pw = if bend > 0.001 { p * (1.0 + bend) / (1.0 + bend * p) } else { p };
                                    tables.sample(bank, oc.p1, level_idx[o], pw)
                                };
                                let y = y * norm;
                                mono += y;
                                let pan = (oc.pan + pos * s(snap, P::UniSpread)).clamp(-1.0, 1.0);
                                let a = (pan + 1.0) * FRAC_PI_4;
                                l += y * a.cos();
                                r += y * a.sin();
                            }
                        }
                        OSC_TYPE_FM => {
                            let osc = &mut self.osc[o];
                            let ratio = RATIOS[((oc.p1 * 15.0).round() as usize).min(15)];
                            let index = oc.p2 * oc.p2 * 9.0;
                            let fb = oc.p3 * 1.2;
                            osc.fm_c += dtp;
                            osc.fm_c -= osc.fm_c.floor();
                            osc.fm_m += dtp * ratio;
                            osc.fm_m -= osc.fm_m.floor();
                            osc.phase[0] = osc.fm_c;
                            let m_out = (osc.fm_m * TAU + fb * osc.fm_prev).sin();
                            osc.fm_prev = m_out;
                            let y = (((osc.fm_c + pm) * TAU) + index * m_out).sin();
                            mono = y;
                            let a = (oc.pan + 1.0) * FRAC_PI_4;
                            l = y * a.cos();
                            r = y * a.sin();
                        }
                        OSC_TYPE_PLUCK => {
                            let osc = &mut self.osc[o];
                            let y = osc.pluck.tick(rate / hz.max(10.0), oc.p1, 1.0 - 10.0f32.powf(-1.5 - 2.5 * oc.p3));
                            mono = y;
                            let a = (oc.pan + 1.0) * FRAC_PI_4;
                            l = y * a.cos();
                            r = y * a.sin();
                        }
                        _ => {
                            let osc = &mut self.osc[o];
                            let white = self.rng.bipolar();
                            let a = 1.0 - 0.97 * oc.p1.powf(0.6);
                            osc.noise_lp += (white - osc.noise_lp) * a;
                            let mut y = osc.noise_lp * (1.0 + 3.0 * oc.p1);
                            if oc.p2 > 0.0 {
                                // crackle: sparse impulses crossfaded in
                                let dust = if self.rng.unit() < 0.002 + 0.02 * oc.p2 { self.rng.bipolar() * 2.5 } else { 0.0 };
                                y += (dust - y) * oc.p2;
                            }
                            if oc.p3 > 0.01 {
                                // tuned noise: a narrow band-pass at the note
                                let q = 0.6 + oc.p3 * oc.p3 * 90.0;
                                let o_ = osc.noise_bp.tick(y, warp(hz.min(0.45 * rate), rate), 1.0 / q);
                                y += (o_.bp * (1.0 / q.sqrt()).max(0.1) * 2.0 - y) * oc.p3.min(1.0);
                            }
                            mono = y * 0.7;
                            let a = (oc.pan + 1.0) * FRAC_PI_4;
                            l = mono * a.cos();
                            r = mono * a.sin();
                        }
                    }
                    if o == 0 {
                        a_mono = mono;
                    }
                    if o == 1 {
                        b_mono = mono;
                    }
                    mix_l += l * oc.level;
                    mix_r += r * oc.level;
                }
                // hard sync: B restarts with A
                if b_sync && a_wrapped && matches!(c.osc[1].kind, OSC_TYPE_ANALOG | OSC_TYPE_WAVETABLE) {
                    for p in self.osc[1].phase.iter_mut() {
                        *p = 0.0;
                    }
                }
                // ring modulation of A and B
                if c.ring > 0.0 {
                    let rm = a_mono * b_mono * c.ring * 1.4;
                    mix_l += rm;
                    mix_r += rm;
                }
                // sub oscillator
                if c.sub > 0.0 {
                    self.sub_phase += base_hz[0] * sub_div * dt;
                    self.sub_phase -= self.sub_phase.floor();
                    let y = if sub_square { pulse(self.sub_phase, base_hz[0] * sub_div * dt, 0.5) * 0.7 } else { (self.sub_phase * TAU).sin() };
                    mix_l += y * c.sub;
                    mix_r += y * c.sub;
                }
                // noise
                if c.noise > 0.0 {
                    let white = self.rng.bipolar();
                    let a = 1.0 - 0.97 * noise_color.powf(0.6);
                    let y = noise_filter(&mut self.mixer_noise, white, a) * (1.0 + 3.0 * noise_color) * 0.7;
                    mix_l += y * c.noise;
                    mix_r += y * c.noise;
                }
                // ---- filters
                let cut1 = self.cut1.tick(c.cut1, cut_coef);
                let cut2 = self.cut2.tick(c.cut2, cut_coef);
                let g1 = warp(cut1, rate);
                let g2 = warp(cut2, rate);
                let chans = if c.stereo { 2 } else { 1 };
                let mut outs = [0.0f32; 2];
                for ch in 0..chans {
                    let x = if ch == 0 { mix_l } else { mix_r };
                    let x = if c.stereo { x } else { 0.5 * (mix_l + mix_r) };
                    let pre = drive_in(x, c.drv1);
                    let y1 = self.filter1(ch, pre, f1_type, g1, cut1, c.res1, rate);
                    let y2 = {
                        let src = if parallel { x } else { y1 };
                        let pre2 = drive_in(src, c.drv2);
                        let o = self.f2[ch].tick(pre2, g2, c.res2, c.drv2);
                        match f2_type {
                            0 => o.y[3],
                            1 => o.y[1],
                            2 => (o.y[1] - o.y[3]) * 2.0,
                            _ => o.input - 4.0 * o.y[0] + 6.0 * o.y[1] - 4.0 * o.y[2] + o.y[3],
                        }
                    };
                    outs[ch] = if parallel { y1 * (1.0 - fmix) + y2 * fmix } else { y2 };
                }
                if !c.stereo {
                    outs[1] = outs[0];
                }
                // ---- amp
                let target = ae * vel_gain * c.amp * 0.5;
                let g = self.gain.tick(target, gain_coef);
                let (mut l, mut r) = (outs[0] * g, outs[1] * g);
                if c.pan != 0.0 {
                    let a = (c.pan + 1.0) * FRAC_PI_4;
                    let (ca, sa) = (a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2);
                    l *= ca;
                    r *= sa;
                }
                if !l.is_finite() || !r.is_finite() {
                    self.kill();
                    return false;
                }
                out_l[i + k] += l;
                out_r[i + k] += r;
                self.level = ae;
                if self.amp_env.idle() && self.pending.is_none() {
                    self.active = false;
                    self.kill_filters();
                    return false;
                }
            }
            i += n;
        }
        true
    }

    fn kill_filters(&mut self) {
        for f in self.f1.iter_mut() {
            f.0.reset();
            f.1.reset();
        }
        for f in self.f2.iter_mut() {
            f.reset();
        }
        for c in self.comb.iter_mut() {
            c.reset();
        }
    }

    #[inline]
    fn filter1(&mut self, ch: usize, x: f32, kind: usize, g: f32, cut: f32, res: f32, rate: f32) -> f32 {
        let k1 = 2.0 - 1.96 * res;
        let (a, b) = &mut self.f1[ch];
        match kind {
            0 => a.tick(x, g, k1).lp,
            1 => {
                let o = a.tick(x, g, k1).lp;
                b.tick(o, g, 1.2).lp
            }
            2 => a.tick(x, g, k1).hp,
            3 => {
                let o = a.tick(x, g, k1).hp;
                b.tick(o, g, 1.2).hp
            }
            4 => a.tick(x, g, k1).bp * k1.max(0.3) * 0.8 + 0.0,
            5 => {
                let o = a.tick(x, g, k1);
                o.lp + o.hp
            }
            6 => {
                let o = a.tick(x, g, k1);
                x + o.bp * (1.8 * res) * k1.max(0.2)
            }
            _ => self.comb[ch].tick(x, rate / cut.max(20.0), (res * 0.97).min(0.97)),
        }
    }
}

/// White noise through the one-pole colouring filter (state in `lp`).
fn noise_filter(lp: &mut f32, white: f32, a: f32) -> f32 {
    *lp += (white - *lp) * a;
    *lp
}

/// A pre-filter drive stage: unity at zero, saturating as it grows.
#[inline]
fn drive_in(x: f32, drive: f32) -> f32 {
    if drive <= 0.001 {
        return x;
    }
    let gain = 1.0 + 8.0 * drive;
    (x * gain).tanh() / gain.sqrt()
}

#[cfg(test)]
mod tests {
    use super::super::store::Params;
    use super::super::tables::tables;
    use super::*;

    const FS: f32 = 48_000.0;

    fn render_note(snap: &Snapshot, note: f32, frames: usize, release_at: Option<usize>) -> (Vec<f32>, Vec<f32>) {
        let mut v = Voice::new(5);
        v.note_on(NoteStart { note, velocity: 1.0, legato: false, retrigger: true }, note as u8, 1, snap, FS);
        let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(512);
            if release_at.is_some_and(|t| done <= t && t < done + n) {
                v.note_off();
            }
            v.render(&mut l[done..done + n], &mut r[done..done + n], snap, FS, tables());
            done += n;
        }
        (l, r)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// The fundamental, by autocorrelation (the lag, between 30 Hz and 2 kHz,
    /// at which the signal best matches itself).
    fn pitch_of(x: &[f32]) -> f32 {
        let n = x.len().min(9600);
        let (lo, hi) = ((FS / 2000.0) as usize, (FS / 30.0) as usize);
        let hi = hi.min(n / 2);
        let corr: Vec<f32> = (0..hi + 1).map(|lag| if lag < lo { 0.0 } else { (0..n - lag).map(|i| x[i] * x[i + lag]).sum() }).collect();
        let peak = corr.iter().cloned().fold(0.0f32, f32::max);
        // the first true peak of the correlation that is nearly as strong as
        // the best one (so a multiple of the period is not chosen), refined
        // by a parabola through its neighbours for sub-sample accuracy
        for lag in lo.max(1)..hi {
            if corr[lag] > peak * 0.9 && corr[lag] >= corr[lag - 1] && corr[lag] > corr[lag + 1] {
                let (a, b, c) = (corr[lag - 1], corr[lag], corr[lag + 1]);
                let denom = a - 2.0 * b + c;
                let off = if denom.abs() > 1e-12 { 0.5 * (a - c) / denom } else { 0.0 };
                return FS / (lag as f32 + off);
            }
        }
        0.0
    }

    fn init() -> Snapshot {
        let p = Params::new();
        p.set(P::F1_Cut, 20_000.0);
        p.set(P::F1_Key, 0.0);
        p.snapshot()
    }

    #[test]
    fn every_oscillator_type_sounds_at_the_note_it_was_played() {
        for (kind, name) in [(0.0, "analog"), (1.0, "wavetable"), (2.0, "fm"), (3.0, "pluck")] {
            let p = Params::new();
            p.set(P::F1_Cut, 20_000.0);
            p.set(P::F1_Key, 0.0);
            p.set(P::A_Type, kind);
            p.set(P::A_P1, if kind == 1.0 { 0.0 } else { 0.0 });
            p.set(P::A_P2, 0.0);
            p.set(P::A_P3, 0.0);
            p.set(P::AmpS, 1.0);
            let (l, _) = render_note(&p.snapshot(), 69.0, 24000, None);
            let body = &l[4800..];
            assert!(rms(body) > 0.02, "{name} sounds: {}", rms(body));
            assert!(body.iter().all(|s| s.is_finite()));
            let hz = pitch_of(body);
            assert!((hz - 440.0).abs() < 8.0, "{name}: played A4, heard {hz:.1} Hz");
        }
    }

    #[test]
    fn noise_is_noise_and_tuned_noise_finds_the_note() {
        let p = Params::new();
        p.set(P::A_Type, 4.0);
        p.set(P::F1_Cut, 20_000.0);
        p.set(P::F1_Key, 0.0);
        p.set(P::AmpS, 1.0);
        let (l, _) = render_note(&p.snapshot(), 69.0, 24000, None);
        assert!(rms(&l[4800..]) > 0.02);
        p.set(P::A_P3, 1.0);
        let (t, _) = render_note(&p.snapshot(), 69.0, 24000, None);
        assert!(rms(&t[4800..]) > 0.005, "tuned noise still sounds");
    }

    #[test]
    fn a_released_voice_goes_quiet_and_frees_itself() {
        let p = Params::new();
        p.set(P::AmpR, 0.05);
        let mut v = Voice::new(1);
        let snap = p.snapshot();
        v.note_on(NoteStart { note: 60.0, velocity: 1.0, legato: false, retrigger: true }, 60, 1, &snap, FS);
        let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
        for _ in 0..20 {
            v.render(&mut l, &mut r, &snap, FS, tables());
        }
        assert!(v.active);
        v.note_off();
        let mut alive = true;
        for _ in 0..40 {
            l.fill(0.0);
            r.fill(0.0);
            alive = v.render(&mut l, &mut r, &snap, FS, tables());
            if !alive {
                break;
            }
        }
        assert!(!alive && !v.active, "the voice ends after its release");
    }

    #[test]
    fn the_filter_removes_the_highs_and_the_envelope_amount_opens_it() {
        let p = Params::new();
        p.set(P::A_P1, 0.667); // saw
        p.set(P::AmpS, 1.0);
        p.set(P::F1_Key, 0.0);
        p.set(P::F1_Cut, 20_000.0);
        let bright = rms(&render_note(&p.snapshot(), 60.0, 24000, None).0[4800..]);
        p.set(P::F1_Cut, 150.0);
        let dark = rms(&render_note(&p.snapshot(), 60.0, 24000, None).0[4800..]);
        assert!(dark < bright * 0.6, "a closed filter is quieter: {dark} vs {bright}");
        // an envelope that opens the filter brings the highs back early on
        p.set(P::F1_Cut, 150.0);
        p.set(P::F1_Env, 1.0);
        p.set(P::E2A, 0.0005);
        p.set(P::E2D, 2.0);
        p.set(P::E2S, 1.0);
        let opened = rms(&render_note(&p.snapshot(), 60.0, 24000, None).0[4800..]);
        assert!(opened > dark * 1.5, "env 2 opens the filter: {opened} vs {dark}");
    }

    #[test]
    fn the_mod_matrix_moves_the_pitch() {
        let p = Params::new();
        p.set(P::F1_Cut, 20_000.0);
        p.set(P::F1_Key, 0.0);
        p.set(P::AmpS, 1.0);
        p.set(P::A_P1, 0.0); // sine
        p.set(P::ModWheel, 1.0);
        p.set(P::M1_Src, SRC_MOD_WHEEL as f32);
        p.set(P::M1_Dst, T_PITCH as f32);
        p.set(P::M1_Amt, 0.5); // +12 semitones
        let hz = pitch_of(&render_note(&p.snapshot(), 69.0, 24000, None).0[4800..]);
        assert!((hz - 880.0).abs() < 12.0, "mod wheel raised it an octave: {hz}");
    }

    #[test]
    fn unison_widens_the_sound_and_spreads_it_across_the_channels() {
        let p = Params::new();
        p.set(P::F1_Cut, 20_000.0);
        p.set(P::F1_Key, 0.0);
        p.set(P::AmpS, 1.0);
        let (l1, r1) = render_note(&p.snapshot(), 57.0, 24000, None);
        assert!(l1.iter().zip(&r1).all(|(a, b)| (a - b).abs() < 1e-6), "one voice is centred");
        p.set(P::Unison, 7.0);
        p.set(P::UniDetune, 0.6);
        p.set(P::UniSpread, 1.0);
        let (l, r) = render_note(&p.snapshot(), 57.0, 24000, None);
        let diff: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum::<f32>() / l.len() as f32;
        assert!(diff > 0.01, "unison spreads left from right: {diff}");
        assert!(rms(&l[4800..]) > 0.05 && rms(&l[4800..]) < 2.0);
    }

    #[test]
    fn velocity_scales_the_level() {
        let p = Params::new();
        p.set(P::VelSens, 1.0);
        p.set(P::AmpS, 1.0);
        let snap = p.snapshot();
        let level = |vel: f32| {
            let mut v = Voice::new(1);
            v.note_on(NoteStart { note: 60.0, velocity: vel, legato: false, retrigger: true }, 60, 1, &snap, FS);
            let (mut l, mut r) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
            v.render(&mut l, &mut r, &snap, FS, tables());
            rms(&l[1000..])
        };
        assert!(level(1.0) > level(0.3) * 2.0);
    }

    #[test]
    fn sync_cross_mod_ring_and_sub_all_make_sound_without_blowing_up() {
        let p = Params::new();
        p.set(P::B_Level, 0.6);
        p.set(P::B_Coarse, 7.0);
        p.set(P::B_Sync, 1.0);
        p.set(P::XMod, 0.5);
        p.set(P::Ring, 0.5);
        p.set(P::SubLevel, 0.6);
        p.set(P::Noise, 0.2);
        p.set(P::AmpS, 1.0);
        let (l, r) = render_note(&p.snapshot(), 48.0, 24000, None);
        assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() < 6.0));
        assert!(rms(&l[4800..]) > 0.05);
        let _ = init();
    }

    #[test]
    fn every_filter_type_passes_audio_and_stays_finite() {
        for t in 0..8 {
            for f2 in 0..4 {
                let p = Params::new();
                p.set(P::F1_Type, t as f32);
                p.set(P::F1_Cut, 1500.0);
                p.set(P::F1_Res, 0.7);
                p.set(P::F2_Type, f2 as f32);
                p.set(P::F2_Cut, 3000.0);
                p.set(P::F2_Res, 0.5);
                p.set(P::AmpS, 1.0);
                for route in [0.0, 1.0] {
                    p.set(P::Route, route);
                    let (l, _) = render_note(&p.snapshot(), 48.0, 12000, None);
                    assert!(l.iter().all(|s| s.is_finite() && s.abs() < 10.0), "F1 {t} F2 {f2} route {route}");
                }
            }
        }
    }
}
