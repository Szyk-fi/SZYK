//! What the listening AI apps (Hum, Mouth Drums, Band Mate) share: an
//! input picker that starts on the device's microphone input, a tap the
//! audio thread uses to bring that input down to the models' 16 kHz, and
//! a ring of the most recent 16 kHz audio the inference worker reads.
//!
//! The split is the one the device will use: the audio thread only copies
//! and filters samples (it never waits on the model), and a separate
//! worker -- the NPU's job on the STM32N6 -- picks up frames from the ring
//! when they're ready.

use crate::apps::neural::Decimator;
use crate::audio_bus::{AudioBus, NO_SOURCE};
use crate::util::AtomicF32;
use std::sync::{Arc, Mutex};

/// About two seconds at 16 kHz.
const RING: usize = 32_768;

struct RingData {
    buf: Vec<f32>,
    /// Samples written since the start.
    written: u64,
}

/// The most recent 16 kHz input.
pub struct Ring {
    data: Mutex<RingData>,
}

impl Default for Ring {
    fn default() -> Self {
        Ring { data: Mutex::new(RingData { buf: vec![0.0; RING], written: 0 }) }
    }
}

impl Ring {
    /// From the audio thread: never waits; if the worker holds the lock
    /// this block is dropped (the worker is quick, so this is rare).
    pub fn write(&self, x: &[f32]) {
        if let Ok(mut d) = self.data.try_lock() {
            for &v in x {
                let i = (d.written % RING as u64) as usize;
                d.buf[i] = v;
                d.written += 1;
            }
        }
    }

    /// For tests: writes, waiting for the lock.
    #[cfg(test)]
    pub fn write_blocking(&self, x: &[f32]) {
        let mut d = self.data.lock().unwrap();
        for &v in x {
            let i = (d.written % RING as u64) as usize;
            d.buf[i] = v;
            d.written += 1;
        }
    }

    pub fn written(&self) -> u64 {
        self.data.lock().map(|d| d.written).unwrap_or(0)
    }

    /// Copies the `out.len()` samples ending at sample `end` (counted from
    /// the start). Returns false if they're no longer (or not yet) there.
    pub fn read_ending_at(&self, end: u64, out: &mut [f32]) -> bool {
        let d = self.data.lock().unwrap();
        let n = out.len() as u64;
        if end > d.written || end < n || d.written - (end - n) > RING as u64 {
            return false;
        }
        for (k, o) in out.iter_mut().enumerate() {
            *o = d.buf[((end - n + k as u64) % RING as u64) as usize];
        }
        true
    }
}

/// The UI side: which input to listen to.
pub struct Listen {
    bus: Arc<AudioBus>,
    pub source: usize,
    buf: Arc<Mutex<Option<Arc<Mutex<Vec<f32>>>>>>,
    poll: u32,
    pub ring: Arc<Ring>,
    /// The input's recent level, 0..1 (from the audio thread).
    pub level: Arc<AtomicF32>,
}

impl Listen {
    pub fn new(bus: Arc<AudioBus>) -> Listen {
        let names = bus.names();
        let source = names.iter().position(|n| n == "Hardware input").or_else(|| names.iter().position(|n| n.contains("Audio In"))).unwrap_or(NO_SOURCE);
        let mut l = Listen { bus, source, buf: Arc::new(Mutex::new(None)), poll: 0, ring: Arc::new(Ring::default()), level: Arc::new(AtomicF32::new(0.0)) };
        l.connect();
        l
    }

    fn connect(&mut self) {
        let b = if self.source == NO_SOURCE { None } else { self.bus.get(self.source) };
        *self.buf.lock().unwrap() = b;
    }

    pub fn step(&mut self, d: i32) {
        self.source = crate::audio_bus::cycle_source(self.source, d.signum(), self.bus.len());
        self.connect();
    }

    pub fn name(&self) -> String {
        self.bus.source_name(self.source)
    }

    pub fn connected(&self) -> bool {
        self.source != NO_SOURCE
    }

    /// Once a frame: keeps the input awake (sources only run while read).
    pub fn tick(&mut self) {
        self.poll += 1;
        if self.poll % 15 == 0 && self.source != NO_SOURCE {
            let _ = self.bus.get(self.source);
        }
    }

    /// The audio thread's end.
    pub fn tap(&self) -> Tap {
        Tap { buf: Arc::clone(&self.buf), dec: Decimator::new(), block: Vec::with_capacity(4096), out: Vec::with_capacity(4096), ring: Arc::clone(&self.ring), level: Arc::clone(&self.level), peak: 0.0 }
    }
}

/// Copies the chosen input into the ring at 16 kHz, once per audio block.
pub struct Tap {
    buf: Arc<Mutex<Option<Arc<Mutex<Vec<f32>>>>>>,
    dec: Decimator,
    block: Vec<f32>,
    out: Vec<f32>,
    ring: Arc<Ring>,
    level: Arc<AtomicF32>,
    peak: f32,
}

impl Tap {
    pub fn process(&mut self, frames: usize, sr: f32) {
        self.block.clear();
        if let Ok(src) = self.buf.try_lock() {
            if let Some(b) = src.as_ref() {
                if let Ok(b) = b.try_lock() {
                    self.block.extend(b.iter().take(frames));
                }
            }
        }
        // A missing or short input block is silence, so time keeps moving.
        self.block.resize(frames, 0.0);
        let p = self.block.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        self.peak = p.max(self.peak * 0.9);
        self.level.set(self.peak);
        self.out.clear();
        self.dec.push(&self.block, sr, &mut self.out);
        self.ring.write(&self.out);
    }
}

/// A worker thread that runs `step` every couple of milliseconds until the
/// returned handle is dropped -- the stand-in for the NPU's job queue.
pub struct Worker {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(name: &str, mut step: impl FnMut() + Send + 'static) -> Worker {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while !s.load(std::sync::atomic::Ordering::Relaxed) {
                    step();
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            })
            .ok();
        Worker { stop, handle }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_returns_the_latest_samples() {
        let r = Ring::default();
        let x: Vec<f32> = (0..50_000).map(|i| i as f32).collect();
        r.write_blocking(&x);
        let mut out = [0.0; 4];
        assert!(r.read_ending_at(50_000, &mut out));
        assert_eq!(out, [49_996.0, 49_997.0, 49_998.0, 49_999.0]);
        assert!(!r.read_ending_at(50_001, &mut out), "not written yet");
        assert!(!r.read_ending_at(10, &mut out), "long gone");
    }

    #[test]
    fn the_tap_brings_the_input_down_to_16k() {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        let l = Listen::new(Arc::clone(&bus));
        assert!(l.connected());
        let mut tap = l.tap();
        for _ in 0..10 {
            *input.lock().unwrap() = vec![0.5; 480];
            tap.process(480, 48_000.0);
        }
        assert_eq!(l.ring.written(), 1600);
        assert!(l.level.get() > 0.4);
    }
}
