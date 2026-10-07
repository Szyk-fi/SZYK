//! The audio side of Blaster: eight voices, the echo and the master stage.
//! It runs on the audio thread and never blocks or allocates after it is
//! built.

use super::params::*;
use super::store::{Params, Snapshot};
use super::voice::{Phase, Voice};
use crate::apps::hydra::dsp::{soft_clip, Delay};
use crate::apps::mi_kit::NoteQueue;
use crate::audio::AudioProcessor;
use crate::util::AtomicF32;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const VOICES: usize = 8;
pub const SCOPE_LEN: usize = 256;
const CHUNK: usize = 64;
/// The longest echo, in samples: 0.8 s at the highest rate the device runs.
const ECHO_MAX: usize = 96_000;

/// External control inputs on the modulation bus.
pub struct Cv {
    /// Octaves added to every pitch (the pitch of notes as they are struck).
    pub pitch: Arc<AtomicF32>,
    /// Scales the charge time by 4^value (so -0.5..0.5 halves or doubles it).
    pub charge: Arc<AtomicF32>,
    /// Scales the blast length the same way.
    pub length: Arc<AtomicF32>,
}

/// What the UI thread and the audio thread share.
pub struct Shared {
    pub params: Params,
    pub queue: Arc<NoteQueue>,
    pub cv: Cv,
    pub mix_level: Arc<AtomicF32>,
    pub ext_mix_level: Arc<AtomicF32>,
    pub output: Arc<Mutex<Vec<f32>>>,
    /// The fullest charge among the voices charging (for the meter).
    pub charge: AtomicF32,
    /// Voices charging or blasting.
    pub active: AtomicUsize,
    /// Counts blasts fired, so the screen can flash.
    pub fired: AtomicU32,
    /// How big the last blast was, 0..1.
    pub last_power: AtomicF32,
    pub peak: AtomicF32,
    pub scope: Mutex<Vec<f32>>,
    pub panic: std::sync::atomic::AtomicBool,
}

pub struct Engine {
    sh: Arc<Shared>,
    voices: Vec<Voice>,
    echo: Delay,
    buf: Vec<f32>,
    clock: u64,
    scope_buf: [f32; SCOPE_LEN],
    scope_pos: usize,
}

impl Engine {
    pub fn new(sh: Arc<Shared>) -> Self {
        Self {
            sh,
            voices: (0..VOICES).map(|i| Voice::new(0x2468_ACE1 ^ (i as u32).wrapping_mul(0x9E37_79B1))).collect(),
            echo: Delay::new(ECHO_MAX),
            buf: vec![0.0; CHUNK],
            clock: 0,
            scope_buf: [0.0; SCOPE_LEN],
            scope_pos: 0,
        }
    }

    /// The values this block renders with: the table plus the control inputs.
    fn cvs(&self) -> (f32, f32, f32) {
        let cv = &self.sh.cv;
        (cv.pitch.get() * 2.0, (cv.charge.get() * 2.0).exp2(), (cv.length.get() * 2.0).exp2())
    }

    /// A voice for a new note: an idle one, else the oldest blast, else the
    /// oldest charge.
    fn alloc(&self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| v.is_idle()) {
            return i;
        }
        let oldest = |phase: Phase| self.voices.iter().enumerate().filter(|(_, v)| v.phase == phase).min_by_key(|(_, v)| v.age).map(|(i, _)| i);
        oldest(Phase::Blasting).or_else(|| oldest(Phase::Charging)).unwrap_or(0)
    }

    fn handle(&mut self, note: u8, velocity: u8, snap: &Snapshot) {
        if velocity > 0 {
            // a key struck again while it is still charging starts afresh
            let i = self.alloc();
            self.clock += 1;
            let (pitch, ct, len) = self.cvs();
            self.voices[i].note_on(note, velocity, snap, pitch, ct, len, self.clock);
        } else if let Some(i) = self.voices.iter().enumerate().filter(|(_, v)| v.phase == Phase::Charging && v.note == note).max_by_key(|(_, v)| v.age).map(|(i, _)| i) {
            let before = self.voices[i].phase;
            self.voices[i].note_off(snap);
            if before == Phase::Charging {
                self.sh.last_power.set(self.voices[i].power());
                self.sh.fired.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl AudioProcessor for Engine {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        if self.sh.panic.swap(false, Ordering::Relaxed) {
            self.voices.iter_mut().for_each(|v| v.kill());
            self.echo.clear();
        }
        // the voices apply the pitch, charge-time and length inputs themselves, as they are struck
        let snap = self.sh.params.snapshot();
        while let Some(e) = self.sh.queue.pop() {
            self.handle(e.note, e.velocity, &snap);
        }
        let frames = out.len() / channels;
        let level = (self.sh.mix_level.get() + self.sh.ext_mix_level.get()).clamp(0.0, 2.0) * snap[P::Level as usize];
        let (echo_mix, echo_fb) = (snap[P::EchoMix as usize], snap[P::EchoFb as usize]);
        let echo_delay = snap[P::EchoTime as usize] * 0.001 * rate;
        let mut mono_out = self.sh.output.try_lock().ok();
        if let Some(m) = mono_out.as_mut() {
            m.clear();
        }
        let mut peak = 0.0f32;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            self.buf[..n].fill(0.0);
            for v in self.voices.iter_mut() {
                let was_charging = v.phase == Phase::Charging;
                v.render(&mut self.buf[..n], &snap, rate);
                // a voice that fired itself at full charge (Fire at full)
                if was_charging && v.phase != Phase::Charging {
                    self.sh.last_power.set(v.power());
                    self.sh.fired.fetch_add(1, Ordering::Relaxed);
                }
            }
            for k in 0..n {
                let dry = self.buf[k];
                let wet = if echo_mix > 0.0 { self.echo.read(echo_delay) } else { 0.0 };
                self.echo.write(dry + wet * echo_fb);
                let y = soft_clip((dry + wet * echo_mix) * level * 1.4);
                let y = if y.is_finite() { y } else { 0.0 };
                peak = peak.max(y.abs());
                let frame = &mut out[(done + k) * channels..(done + k + 1) * channels];
                for c in frame.iter_mut() {
                    *c += y;
                }
                if let Some(m) = mono_out.as_mut() {
                    m.push(y);
                }
                self.scope_buf[self.scope_pos] = y;
                self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
            }
            done += n;
        }
        drop(mono_out);
        self.sh.peak.set(peak);
        self.sh.charge.set(self.voices.iter().map(|v| v.charge()).fold(0.0, f32::max));
        self.sh.active.store(self.voices.iter().filter(|v| !v.is_idle()).count(), Ordering::Relaxed);
        if let Ok(mut s) = self.sh.scope.try_lock() {
            if s.len() == SCOPE_LEN {
                for (i, o) in s.iter_mut().enumerate() {
                    *o = self.scope_buf[(self.scope_pos + i) % SCOPE_LEN];
                }
            }
        }
    }
}
