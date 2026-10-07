//! The audio side of Hydra: sixteen voices, voice allocation (poly, mono and
//! legato), the arpeggiator, the effects chain and the master stage. It runs
//! on the audio thread and never blocks, locks for long or allocates: every
//! buffer is sized when it is built.

use super::dsp::soft_clip;
use super::fx::Fx;
use super::params::*;
use super::store::{from_norm, to_norm, Params, Snapshot};
use crate::app::music_scales;
use super::tables::{tables, Tables};
use super::voice::{NoteStart, Voice};
use crate::apps::mi_kit::{NoteEvent, NoteQueue};
use crate::audio::AudioProcessor;
use crate::util::AtomicF32;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_VOICES: usize = 16;
/// Notes the engine tracks as physically held (more are ignored).
const HELD: usize = 32;
/// Most notes one key can turn into (a scale ninth is five).
const MAX_CHORD: usize = 6;
/// The audio is rendered in chunks this long, which is also how finely the
/// arpeggiator keeps time (about 1.3 ms at 48 kHz).
const CHUNK: usize = 64;
pub const SCOPE_LEN: usize = 256;
/// Beats per arpeggio step for each Arp_Rate choice.
const ARP_BEATS: [f64; 6] = [1.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125];

/// External control inputs on the modulation bus.
pub struct Cv {
    pub cutoff: Arc<AtomicF32>,
    pub cutoff2: Arc<AtomicF32>,
    pub reso: Arc<AtomicF32>,
    pub drive: Arc<AtomicF32>,
    pub pitch: Arc<AtomicF32>,
    pub wheel: Arc<AtomicF32>,
    pub detune: Arc<AtomicF32>,
    pub delay: Arc<AtomicF32>,
    pub reverb: Arc<AtomicF32>,
    pub macros: [Arc<AtomicF32>; 4],
}

/// What the UI thread and the audio thread share.
pub struct Shared {
    pub params: Params,
    pub queue: Arc<NoteQueue>,
    pub cv: Cv,
    pub mix_level: Arc<AtomicF32>,
    pub ext_mix_level: Arc<AtomicF32>,
    pub output: Arc<Mutex<Vec<f32>>>,
    /// Voices sounding, for the display.
    pub active: AtomicUsize,
    pub peak: AtomicF32,
    pub scope: Mutex<Vec<f32>>,
    /// The note the arpeggiator is on (255 = none).
    pub arp_now: AtomicUsize,
    /// A request to release everything (set by the UI, taken by the engine).
    pub panic: std::sync::atomic::AtomicBool,
}

#[derive(Clone, Copy, Default)]
struct Held {
    note: u8,
    velocity: u8,
}

pub struct Engine {
    sh: Arc<Shared>,
    voices: Vec<Voice>,
    fx: Fx,
    tables: &'static Tables,
    left: Vec<f32>,
    right: Vec<f32>,
    clock: u64,
    held: [Held; HELD],
    held_n: usize,
    /// Keys physically down (as opposed to `held`, which Hold keeps latched).
    down: [bool; 128],
    down_n: usize,
    latched: [bool; 128],
    /// What each struck key turned into, so letting go stops exactly those
    /// notes even if the Chord setting has changed since.
    chord_of: [[u8; MAX_CHORD]; 128],
    chord_len: [u8; 128],
    /// How many struck keys are sounding each note (chords overlap).
    sounding: [u8; 128],
    // arpeggiator
    arp_pos: f64,
    arp_next: f64,
    arp_step: i64,
    arp_idx: usize,
    arp_dir: i32,
    arp_note: Option<u8>,
    arp_off_at: f64,
    arp_was_on: bool,
    rng: u32,
    scope_pos: usize,
    scope_buf: [f32; SCOPE_LEN],
}

impl Engine {
    pub fn new(sh: Arc<Shared>) -> Self {
        Self {
            sh,
            voices: (0..MAX_VOICES).map(|i| Voice::new(0x1234_5678 ^ (i as u32).wrapping_mul(0x9E37_79B1))).collect(),
            fx: Fx::new(),
            tables: tables(),
            left: vec![0.0; CHUNK],
            right: vec![0.0; CHUNK],
            clock: 0,
            held: [Held::default(); HELD],
            held_n: 0,
            down: [false; 128],
            down_n: 0,
            latched: [false; 128],
            chord_of: [[0; MAX_CHORD]; 128],
            chord_len: [0; 128],
            sounding: [0; 128],
            arp_pos: 0.0,
            arp_next: 0.0,
            arp_step: 0,
            arp_idx: 0,
            arp_dir: 1,
            arp_note: None,
            arp_off_at: 0.0,
            arp_was_on: false,
            rng: 0x2545_f491,
            scope_pos: 0,
            scope_buf: [0.0; SCOPE_LEN],
        }
    }

    /// The parameters this block renders with: the table's values plus what
    /// is patched into the external inputs.
    pub(super) fn snapshot(&self) -> Snapshot {
        let mut s = self.sh.params.snapshot();
        let cv = &self.sh.cv;
        let at = |p: P| p as usize;
        s[at(P::F1_Cut)] = (s[at(P::F1_Cut)] * (cv.cutoff.get() * 5.0).exp2()).clamp(16.0, 22_000.0);
        s[at(P::F2_Cut)] = (s[at(P::F2_Cut)] * (cv.cutoff2.get() * 5.0).exp2()).clamp(16.0, 22_000.0);
        s[at(P::F1_Res)] = (s[at(P::F1_Res)] + cv.reso.get()).clamp(0.0, 1.0);
        s[at(P::Drv_Amt)] = (s[at(P::Drv_Amt)] + cv.drive.get()).clamp(0.0, 1.0);
        s[at(P::Tune)] += cv.pitch.get() * 1200.0;
        s[at(P::ModWheel)] = (s[at(P::ModWheel)] + cv.wheel.get()).clamp(0.0, 1.0);
        s[at(P::UniDetune)] = (s[at(P::UniDetune)] + cv.detune.get()).clamp(0.0, 1.0);
        s[at(P::Dly_Mix)] = (s[at(P::Dly_Mix)] + cv.delay.get()).clamp(0.0, 1.0);
        s[at(P::Rev_Mix)] = (s[at(P::Rev_Mix)] + cv.reverb.get()).clamp(0.0, 1.0);
        for (i, m) in cv.macros.iter().enumerate() {
            let k = at(P::Mac1) + i;
            s[k] = (s[k] + m.get()).clamp(0.0, 1.0);
        }
        // Macros push their destinations from where the sound has them.
        for m in 0..4 {
            let pos = s[at(P::Mac1) + m];
            if pos <= 0.0 {
                continue;
            }
            for slot in 0..MACRO_SLOTS {
                let base = at(P::Mac1_DA) + m * 4 + slot * 2;
                let Some(dest) = macro_dest(s[base] as usize) else { continue };
                let i = dest as usize;
                s[i] = from_norm(&DEFS[i], to_norm(&DEFS[i], s[i]) + pos * s[base + 1]);
            }
        }
        s
    }

    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as usize % n.max(1)
    }

    // ------------------------------------------------------------ notes

    fn add_held(&mut self, note: u8, velocity: u8) {
        if let Some(h) = self.held[..self.held_n].iter_mut().find(|h| h.note == note) {
            h.velocity = velocity;
            return;
        }
        if self.held_n < HELD {
            self.held[self.held_n] = Held { note, velocity };
            self.held_n += 1;
        }
    }

    fn remove_held(&mut self, note: u8) {
        if let Some(i) = self.held[..self.held_n].iter().position(|h| h.note == note) {
            self.held.copy_within(i + 1..self.held_n, i);
            self.held_n -= 1;
        }
    }

    fn alloc(&self, nv: usize) -> usize {
        if let Some(i) = (0..nv).find(|&i| !self.voices[i].active && self.voices[i].pending.is_none()) {
            return i;
        }
        if let Some(i) = (0..nv).filter(|&i| self.voices[i].releasing()).min_by(|&a, &b| self.voices[a].level.total_cmp(&self.voices[b].level)) {
            return i;
        }
        (0..nv).min_by_key(|&i| self.voices[i].age).unwrap_or(0)
    }

    fn start_note(&mut self, note: u8, velocity: u8, snap: &Snapshot, rate: f32) {
        self.clock += 1;
        let ns = NoteStart { note: note as f32, velocity: velocity as f32 / 127.0, legato: false, retrigger: true };
        let mode = snap[P::Mode as usize] as usize;
        let nv = (snap[P::Voices as usize] as usize).clamp(1, MAX_VOICES);
        if mode == 0 {
            // the same key struck again takes its own voice back
            let i = (0..MAX_VOICES).find(|&i| self.voices[i].active && self.voices[i].key == note && self.voices[i].pending.is_none()).unwrap_or_else(|| self.alloc(nv));
            let age = self.clock;
            let v = &mut self.voices[i];
            if v.active && v.key != note {
                v.steal_for(ns, note);
            } else {
                v.note_on(ns, note, age, snap, rate);
            }
        } else {
            let age = self.clock;
            let legato_note = NoteStart { legato: true, retrigger: mode == 1, ..ns };
            let v = &mut self.voices[0];
            if v.active && v.gate {
                v.note_on(legato_note, note, age, snap, rate);
            } else if v.active {
                v.note_on(legato_note, note, age, snap, rate);
            } else {
                v.note_on(ns, note, age, snap, rate);
            }
        }
    }

    fn release_note(&mut self, note: u8, snap: &Snapshot, rate: f32) {
        let mode = snap[P::Mode as usize] as usize;
        if mode == 0 {
            for v in self.voices.iter_mut() {
                if v.active && v.gate && v.key == note {
                    v.note_off();
                }
                if v.pending.is_some() && v.pending_key == note {
                    v.pending = None;
                }
            }
        } else if self.voices[0].key == note || self.held_n == 0 {
            if self.held_n > 0 {
                // fall back to the last note still held
                let h = self.held[self.held_n - 1];
                let age = self.clock;
                let ns = NoteStart { note: h.note as f32, velocity: h.velocity as f32 / 127.0, legato: true, retrigger: mode == 1 };
                self.voices[0].note_on(ns, h.note, age, snap, rate);
            } else {
                self.voices[0].note_off();
            }
        }
    }

    fn all_off(&mut self) {
        for v in self.voices.iter_mut() {
            if v.active {
                v.note_off();
            }
        }
        self.held_n = 0;
        self.down = [false; 128];
        self.down_n = 0;
        self.latched = [false; 128];
        self.chord_len = [0; 128];
        self.sounding = [0; 128];
        self.arp_note = None;
    }

    /// The notes a struck key plays: itself, or its chord.
    pub(super) fn expand(note: u8, snap: &Snapshot, out: &mut [u8; MAX_CHORD]) -> usize {
        let chord = snap[P::Chord as usize] as usize;
        let mut offsets = [0i32; MAX_CHORD];
        let mut n = 1;
        if chord >= CHORD_SCALE_FIRST {
            // Stack scale degrees (every other one) on the key's own degree.
            let scale = snap[P::Scale as usize] as usize;
            let root = snap[P::Root as usize] as i32;
            let steps = music_scales::intervals(scale);
            let rel = (note as i32 - root).rem_euclid(12);
            let deg = steps.iter().rposition(|&i| i <= rel).unwrap_or(0);
            let base = music_scales::degree(scale, deg);
            n = 3 + (chord - CHORD_SCALE_FIRST).min(2);
            for k in 0..n {
                offsets[k] = music_scales::degree(scale, deg + 2 * k) - base;
            }
        } else if chord > 0 {
            let shape = CHORD_SHAPES[chord.min(CHORD_SHAPES.len() - 1)];
            n = shape.len().min(MAX_CHORD);
            offsets[..n].copy_from_slice(&shape[..n]);
        }
        let mut count = 0;
        for &o in &offsets[..n] {
            let m = note as i32 + o;
            if (0..128).contains(&m) {
                out[count] = m as u8;
                count += 1;
            }
        }
        count
    }

    /// Strikes a key: its chord plays when the voices are poly (a mono
    /// voice has only one note to give).
    fn strike(&mut self, note: u8, velocity: u8, snap: &Snapshot, rate: f32) {
        let mut notes = [0u8; MAX_CHORD];
        let n = if snap[P::Mode as usize] as usize == 0 { Self::expand(note, snap, &mut notes) } else { notes[0] = note; 1 };
        self.chord_of[note as usize] = notes;
        self.chord_len[note as usize] = n as u8;
        for &m in &notes[..n] {
            self.sounding[m as usize] = self.sounding[m as usize].saturating_add(1);
            self.start_note(m, velocity, snap, rate);
        }
    }

    fn let_go(&mut self, note: u8, snap: &Snapshot, rate: f32) {
        let n = self.chord_len[note as usize] as usize;
        if n == 0 {
            self.release_note(note, snap, rate);
            return;
        }
        self.chord_len[note as usize] = 0;
        let notes = self.chord_of[note as usize];
        for &m in &notes[..n] {
            let c = &mut self.sounding[m as usize];
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.release_note(m, snap, rate);
            }
        }
    }

    /// Lets go of everything Hold has been keeping.
    fn release_latched(&mut self, snap: &Snapshot, rate: f32) {
        for note in 0..128u8 {
            if self.latched[note as usize] {
                self.latched[note as usize] = false;
                self.remove_held(note);
                if snap[P::Arp_On as usize] < 0.5 {
                    self.let_go(note, snap, rate);
                }
            }
        }
    }

    fn handle(&mut self, e: NoteEvent, snap: &Snapshot, rate: f32) {
        let arp = snap[P::Arp_On as usize] >= 0.5;
        let hold = snap[P::Hold as usize] >= 0.5;
        let k = (e.note as usize).min(127);
        if e.velocity > 0 {
            // A fresh hand on the keys replaces what Hold was keeping.
            if hold && self.down_n == 0 {
                self.release_latched(snap, rate);
            }
            if !self.down[k] {
                self.down[k] = true;
                self.down_n += 1;
            }
            self.latched[k] = false;
            self.add_held(e.note, e.velocity);
            if !arp {
                // striking a key that is still sounding retriggers cleanly
                if self.chord_len[k] > 0 {
                    self.let_go(e.note, snap, rate);
                }
                self.strike(e.note, e.velocity, snap, rate);
            }
        } else {
            if self.down[k] {
                self.down[k] = false;
                self.down_n -= 1;
            }
            if hold {
                self.latched[k] = true;
                return;
            }
            self.remove_held(e.note);
            if !arp {
                self.let_go(e.note, snap, rate);
            }
        }
    }

    // --------------------------------------------------------- arpeggio

    fn arp_pool(&self, snap: &Snapshot, pool: &mut [u8; 128]) -> usize {
        let octaves = (snap[P::Arp_Oct as usize] as usize).clamp(1, 4);
        let mode = snap[P::Arp_Mode as usize] as usize;
        // the held keys, each as its chord (so the arpeggio runs through it)
        let mut notes = [0u8; HELD * MAX_CHORD];
        let mut n = 0;
        for i in 0..self.held_n {
            let mut chord = [0u8; MAX_CHORD];
            let c = Self::expand(self.held[i].note, snap, &mut chord);
            for &m in &chord[..c] {
                if !notes[..n].contains(&m) {
                    notes[n] = m;
                    n += 1;
                }
            }
        }
        if mode != 4 {
            notes[..n].sort_unstable();
        }
        let mut count = 0;
        for o in 0..octaves {
            for &note in &notes[..n] {
                let m = note as usize + o * 12;
                if m < 128 && count < 128 {
                    pool[count] = m as u8;
                    count += 1;
                }
            }
        }
        count
    }

    fn arp_pick(&mut self, mode: usize, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        match mode {
            0 | 4 => {
                self.arp_idx = (self.arp_idx + 1) % count;
                self.arp_idx
            }
            1 => {
                self.arp_idx = (self.arp_idx + count - 1) % count;
                self.arp_idx
            }
            2 => {
                if count == 1 {
                    return 0;
                }
                let next = self.arp_idx as i32 + self.arp_dir;
                if next < 0 || next >= count as i32 {
                    self.arp_dir = -self.arp_dir;
                }
                self.arp_idx = (self.arp_idx as i32 + self.arp_dir).clamp(0, count as i32 - 1) as usize;
                self.arp_idx
            }
            _ => self.rand(count),
        }
    }

    fn arp_release(&mut self, snap: &Snapshot, rate: f32) {
        if let Some(n) = self.arp_note.take() {
            self.release_note_force(n, snap, rate);
        }
        self.sh.arp_now.store(255, Ordering::Relaxed);
    }

    /// Releases a note the arpeggiator started (it is not in `held`).
    fn release_note_force(&mut self, note: u8, snap: &Snapshot, _rate: f32) {
        if snap[P::Mode as usize] as usize == 0 {
            for v in self.voices.iter_mut() {
                if v.active && v.gate && v.key == note {
                    v.note_off();
                }
            }
        } else {
            self.voices[0].note_off();
        }
    }

    fn run_arp(&mut self, frames: usize, snap: &Snapshot, rate: f32) {
        let on = snap[P::Arp_On as usize] >= 0.5;
        if !on {
            if self.arp_was_on {
                self.arp_release(snap, rate);
                self.arp_was_on = false;
                // anything still held plays as a normal chord
                self.chord_len = [0; 128];
                self.sounding = [0; 128];
                for i in 0..self.held_n {
                    let h = self.held[i];
                    self.strike(h.note, h.velocity, snap, rate);
                }
            }
            return;
        }
        if !self.arp_was_on {
            // switching the arpeggiator on: stop the held chord, then arpeggiate it
            self.arp_was_on = true;
            self.chord_len = [0; 128];
            self.sounding = [0; 128];
            for v in self.voices.iter_mut() {
                if v.active && v.gate {
                    v.note_off();
                }
            }
            self.arp_pos = 0.0;
            self.arp_next = 0.0;
            self.arp_step = 0;
            self.arp_idx = usize::MAX;
        }
        if self.held_n == 0 {
            self.arp_release(snap, rate);
            self.arp_pos = 0.0;
            self.arp_next = 0.0;
            self.arp_step = 0;
            self.arp_idx = usize::MAX;
            return;
        }
        let bpm = snap[P::Arp_Bpm as usize].clamp(40.0, 240.0) as f64;
        let beats = ARP_BEATS[(snap[P::Arp_Rate as usize] as usize).min(ARP_BEATS.len() - 1)];
        let step_len = 60.0 / bpm * rate as f64 * beats;
        let swing = snap[P::Arp_Swing as usize] as f64;
        let gate = snap[P::Arp_Gate as usize] as f64;
        let end = self.arp_pos + frames as f64;
        // a gate that ends inside this chunk
        if self.arp_note.is_some() && self.arp_off_at <= end {
            self.arp_release(snap, rate);
        }
        while self.arp_next < end {
            self.arp_release(snap, rate);
            let mut pool = [0u8; 128];
            let count = self.arp_pool(snap, &mut pool);
            if count > 0 {
                let mode = snap[P::Arp_Mode as usize] as usize;
                if self.arp_idx == usize::MAX {
                    self.arp_idx = if mode == 1 { 0 } else { count - 1 };
                }
                if self.arp_idx >= count {
                    self.arp_idx %= count;
                }
                let i = self.arp_pick(mode, count);
                let note = pool[i.min(count - 1)];
                let vel = self.held[self.held_n - 1].velocity;
                self.start_note(note, vel, snap, rate);
                self.arp_note = Some(note);
                self.sh.arp_now.store(note as usize, Ordering::Relaxed);
                self.arp_off_at = self.arp_next + gate * step_len;
            }
            self.arp_step += 1;
            let swung = if self.arp_step % 2 == 1 { swing * step_len } else { 0.0 };
            self.arp_next = self.arp_step as f64 * step_len + swung;
        }
        self.arp_pos = end;
    }
}

impl AudioProcessor for Engine {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        if channels == 0 || rate < 8000.0 || rate > 200_000.0 {
            return;
        }
        if self.sh.panic.swap(false, Ordering::Relaxed) {
            self.all_off();
            for v in self.voices.iter_mut() {
                v.kill();
            }
        }
        let snap = self.snapshot();
        while let Some(e) = self.sh.queue.pop() {
            self.handle(e, &snap, rate);
        }
        if snap[P::Hold as usize] < 0.5 && self.latched.iter().any(|&l| l) {
            self.release_latched(&snap, rate);
        }
        let level = (self.sh.mix_level.get() + self.sh.ext_mix_level.get()).clamp(0.0, 2.0) * snap[P::Level as usize];
        let frames = out.len() / channels;
        let mut peak = 0.0f32;
        let mut done = 0;
        let sh = Arc::clone(&self.sh);
        let mut mono_out = sh.output.try_lock().ok();
        if let Some(m) = mono_out.as_mut() {
            m.clear();
        }
        while done < frames {
            let n = (frames - done).min(CHUNK);
            self.run_arp(n, &snap, rate);
            self.left[..n].fill(0.0);
            self.right[..n].fill(0.0);
            let nv_cap = MAX_VOICES;
            for vi in 0..nv_cap {
                if !self.voices[vi].active {
                    continue;
                }
                self.voices[vi].render(&mut self.left[..n], &mut self.right[..n], &snap, rate, self.tables);
            }
            self.fx.process(&mut self.left[..n], &mut self.right[..n], &snap, rate);
            for k in 0..n {
                let l = soft_clip(self.left[k] * level * 1.4);
                let r = soft_clip(self.right[k] * level * 1.4);
                let (l, r) = if l.is_finite() && r.is_finite() { (l, r) } else { (0.0, 0.0) };
                peak = peak.max(l.abs()).max(r.abs());
                let frame = &mut out[(done + k) * channels..(done + k + 1) * channels];
                if channels == 1 {
                    frame[0] += 0.5 * (l + r);
                } else {
                    frame[0] += l;
                    frame[1] += r;
                }
                let mono = 0.5 * (l + r);
                if let Some(m) = mono_out.as_mut() {
                    m.push(mono);
                }
                self.scope_buf[self.scope_pos] = mono;
                self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
            }
            done += n;
        }
        drop(mono_out);
        self.sh.peak.set(peak);
        self.sh.active.store(self.voices.iter().filter(|v| v.active).count(), Ordering::Relaxed);
        if let Ok(mut s) = self.sh.scope.try_lock() {
            if s.len() == SCOPE_LEN {
                for (i, o) in s.iter_mut().enumerate() {
                    *o = self.scope_buf[(self.scope_pos + i) % SCOPE_LEN];
                }
            }
        }
    }
}
