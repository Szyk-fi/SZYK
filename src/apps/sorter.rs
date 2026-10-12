//! Sorter (AI): your sample library, sorted by what each sound *is*.
//!
//! Every WAV in the library is heard by a small network on the NPU
//! (`assets/npu/sorter_embed.pmxn` + `sorter_head.pmxn`, trained by
//! `tools/npu/train_sorter.py`): its first ~300 ms becomes a 32-band mel
//! spectrogram, the first network turns that into 48 numbers that place
//! it in a "sounds like" space, and the second names it -- kick, snare,
//! clap, closed or open hat, cymbal, tom, rim, metal, shaker, hand
//! percussion, bass, tonal or texture.
//!
//! Then:
//! * MAP lays the whole library out on screen, similar sounds together
//!   (the embedding's two main directions). The D-pad walks to the
//!   nearest sound that way; the pads play the 16 sounds most like the
//!   one you're on. Knob 1 zooms (1x to 96x) and the view follows the
//!   sound you're on, so a library of ten thousand is walked a
//!   neighbourhood at a time; a corner minimap shows where you are. Each
//!   sound is a soft orb sized by its length (a ring for a loop), lit
//!   additively so crowds glow instead of clogging.
//! * LIST goes through one kind at a time, the most typical first.
//! * KIT builds a 16-pad kit around the sound you're on: for every slot
//!   (kick, snare, hats, ...) the sample of that kind that sounds most
//!   like it, so the kit hangs together. Export copies it into
//!   `media/Kits/`, where Chop, the Sample Drum and the Sequencer find it.
//!
//! The network was trained only on synthesized sounds; its tests check
//! it on drums from generators it never saw. It's good at drum one-shots
//! and broad strokes (a bass note vs a pad); it isn't a genre tagger,
//! and a loop is named by its first hit.
//!
//! The analysis runs on a worker (the NPU's job), and is remembered in
//! `saves/sorter/index.json` so a library is only heard once.

use crate::app::{App, Input, SlintExtra};
use crate::apps::ai_input::Worker;
use crate::apps::kids_kit::{self as kit, Extra, Size2, Sound};
use crate::apps::neural::{self, Decimator, Load, Model, Spectrum};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Sorter";
static EMBED_MODEL: &[u8] = include_bytes!("../../assets/npu/sorter_embed.pmxn");
static HEAD_MODEL: &[u8] = include_bytes!("../../assets/npu/sorter_head.pmxn");
const SR16: f32 = 16_000.0;
const FRAME: usize = 512;
const HOP: usize = 192;
const FRAMES: usize = 24;
const MELS: usize = 32;
const PRE: usize = 1;
const FLOOR: f32 = -10.0;
pub const CLIP: usize = FRAME + HOP * (FRAMES - 1);
pub const EMBED: usize = 48;
pub const CLASSES: [&str; 14] = ["kick", "snare", "clap", "closed hat", "open hat", "cymbal", "tom", "rim", "metal", "shaker", "hand perc", "bass", "tonal", "texture"];
const N_CLASSES: usize = CLASSES.len();
/// LIST's extra page after the kinds: everything heard as a loop.
const LOOPS: usize = N_CLASSES;

fn kind_name(c: usize) -> &'static str {
    if c == LOOPS {
        "loops"
    } else {
        CLASSES[c % N_CLASSES]
    }
}
/// The most files one scan will take. A big sample library is the point.
const MAX_FILES: usize = 50_000;
/// Where the map sits on screen, and the legend beside it.
const MAP_X: i32 = 12;
const MAP_Y: i32 = 38;
const MAP_W: usize = 496;
const MAP_H: usize = 272;
const LEGEND_X: i32 = MAP_X + MAP_W as i32 + 8;
/// The most the map zooms in.
const MAX_ZOOM: f32 = 96.0;
/// The longest file analysed (and auditioned), seconds.
const MAX_SECS: f32 = 20.0;

const COLORS: [(u8, u8, u8); N_CLASSES] = [
    (240, 80, 70),
    (250, 160, 60),
    (250, 205, 80),
    (200, 235, 90),
    (120, 220, 120),
    (80, 215, 190),
    (90, 170, 245),
    (130, 130, 250),
    (190, 120, 245),
    (240, 120, 210),
    (210, 150, 110),
    (150, 110, 90),
    (240, 240, 240),
    (130, 140, 160),
];

fn class_rgb(c: usize) -> [f32; 3] {
    let (r, g, b) = COLORS[c % N_CLASSES];
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// 12345 as "12,345".
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A soft orb: light that falls off smoothly to the edge, added to what's
/// there, so overlapping sounds glow brighter instead of covering each other.
fn splat(buf: &mut [[f32; 3]], x: f32, y: f32, r: f32, c: [f32; 3], gain: f32) {
    let (ix, iy, ri) = (x.round() as i32, y.round() as i32, r.ceil() as i32);
    for py in (iy - ri).max(0)..=(iy + ri).min(MAP_H as i32 - 1) {
        for px in (ix - ri).max(0)..=(ix + ri).min(MAP_W as i32 - 1) {
            let d = ((px as f32 - x).powi(2) + (py as f32 - y).powi(2)).sqrt();
            if d <= r {
                let f = 1.0 - d / r;
                let w = f * f.sqrt() * gain;
                let b = &mut buf[py as usize * MAP_W + px as usize];
                b[0] += c[0] * w;
                b[1] += c[1] * w;
                b[2] += c[2] * w;
            }
        }
    }
}

/// A ring: how a loop (a phrase, not a single hit) is told from a one-shot.
fn splat_ring(buf: &mut [[f32; 3]], x: f32, y: f32, r: f32, c: [f32; 3], gain: f32) {
    let (ix, iy, ri) = (x.round() as i32, y.round() as i32, r.ceil() as i32 + 1);
    for py in (iy - ri).max(0)..=(iy + ri).min(MAP_H as i32 - 1) {
        for px in (ix - ri).max(0)..=(ix + ri).min(MAP_W as i32 - 1) {
            let d = ((px as f32 - x).powi(2) + (py as f32 - y).powi(2)).sqrt();
            let w = (1.0 - (d - r).abs() / 0.9).max(0.0) * gain * 1.2;
            if w > 0.0 {
                let b = &mut buf[py as usize * MAP_W + px as usize];
                b[0] += c[0] * w;
                b[1] += c[1] * w;
                b[2] += c[2] * w;
            }
        }
    }
}

/// The part of the segment a-b inside 0..=w, 0..=h, if any (Liang-Barsky).
fn clip_line(a: (f32, f32), b: (f32, f32), w: f32, h: f32) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a.0), (dx, w - a.0), (-dy, a.1), (dy, h - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return None;
                }
                t1 = t1.min(r);
            }
        }
    }
    Some(((a.0 + t0 * dx, a.1 + t0 * dy), (a.0 + t1 * dx, a.1 + t1 * dy)))
}

fn class_color(c: usize) -> Rgb565 {
    let (r, g, b) = COLORS[c % N_CLASSES];
    kit::rgb(r, g, b)
}

/// The kit KIT builds: the kind of sound on each pad (pad 1 at top left).
const KIT_SLOTS: [usize; 16] = [0, 1, 3, 4, 2, 7, 6, 6, 9, 8, 10, 5, 0, 1, 11, 12];

/// Triangular mel filters on a 512-point FFT, as `features.mel_filters(32, 512)`.
fn mel_filters() -> Vec<Vec<f32>> {
    let hz_to_mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_to_hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    let (lo, hi) = (hz_to_mel(60.0), hz_to_mel(7600.0));
    let pts: Vec<f64> = (0..MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MELS + 1) as f64)).collect();
    (0..MELS)
        .map(|m| {
            let (a, c, b) = (pts[m], pts[m + 1], pts[m + 2]);
            (0..FRAME / 2 + 1)
                .map(|k| {
                    let f = k as f64 * SR16 as f64 / FRAME as f64;
                    ((f - a) / (c - a)).min((b - f) / (b - c)).max(0.0) as f32
                })
                .collect()
        })
        .collect()
}

/// [32 mels, 24 frames], channel-major, as `features.sort_features`.
pub struct Features {
    spec: Spectrum,
    fb: Vec<Vec<f32>>,
}

impl Features {
    pub fn new() -> Features {
        Features { spec: Spectrum::new(FRAME, FRAME), fb: mel_filters() }
    }
    pub fn compute(&mut self, clip: &[f32], out: &mut [f32]) {
        for t in 0..FRAMES {
            let mag = self.spec.compute(&clip[t * HOP..t * HOP + FRAME]);
            for m in 0..MELS {
                out[m * FRAMES + t] = self.fb[m].iter().zip(mag.iter()).map(|(w, x)| w * x).sum();
            }
        }
        let peak = out[..MELS * FRAMES].iter().cloned().fold(0.0f32, f32::max).max(1e-9);
        for v in out[..MELS * FRAMES].iter_mut() {
            *v = ((*v / peak).max(1e-9)).ln().max(FLOOR);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub class: usize,
    pub probs: Vec<f32>,
    /// Unit length.
    pub emb: Vec<f32>,
    pub secs: f32,
    /// Hits found across the whole file (spectral-flux onsets).
    #[serde(default)]
    pub hits: u32,
    /// Several hits spread over its length: a loop or phrase, named by its
    /// first hit but kept out of the kit's one-shot pads.
    #[serde(default)]
    pub is_loop: bool,
}

impl Analysis {
    /// Sure enough of the name to put it on a kit pad.
    pub fn confident(&self) -> bool {
        self.probs.get(self.class).copied().unwrap_or(0.0) >= 0.5
    }
}

/// Hits in a 16 kHz signal: frames whose rise in log-spectral energy
/// (half-wave-rectified spectral flux) stands well above the local
/// average, the noise floor and a tenth of the file's strongest attack,
/// at least 80 ms apart. Returns the hit
/// times in seconds.
pub fn find_hits(x16: &[f32]) -> Vec<f32> {
    const N: usize = 512;
    const H: usize = 256;
    if x16.len() < N * 2 {
        return if x16.iter().any(|v| v.abs() > 1e-4) { vec![0.0] } else { Vec::new() };
    }
    let mut spec = Spectrum::new(N, N);
    let mut prev: Vec<f32> = vec![0.0; N / 2 + 1];
    let mut flux = Vec::new();
    let mut energy = Vec::new();
    let mut t = 0;
    while t + N <= x16.len() {
        let mag = spec.compute(&x16[t..t + N]);
        let mut f = 0.0f32;
        let mut e = 0.0f32;
        for (k, &m) in mag.iter().enumerate() {
            let l = (1.0 + 100.0 * m).ln();
            f += (l - prev[k]).max(0.0);
            prev[k] = l;
            e += m * m;
        }
        flux.push(f);
        energy.push(e);
        t += H;
    }
    let peak_e = energy.iter().cloned().fold(0.0f32, f32::max).max(1e-12);
    // A hit must also be a real attack for this file -- a tenth of its
    // strongest -- or a held note's vibrato counts as a stream of hits.
    let peak_f = flux.iter().cloned().fold(0.0f32, f32::max);
    let mut hits = Vec::new();
    let mut last: Option<usize> = None;
    for i in 0..flux.len() {
        let lo = i.saturating_sub(8);
        let hi = (i + 9).min(flux.len());
        let mean = flux[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        let local_max = flux[i.saturating_sub(2)..(i + 3).min(flux.len())].iter().all(|&v| v <= flux[i]);
        let loud = energy[i.min(energy.len() - 1)..(i + 3).min(energy.len())].iter().cloned().fold(0.0f32, f32::max) > peak_e * 1e-3;
        if local_max && loud && flux[i] > 1.5 * mean + 1.0 && flux[i] >= 0.1 * peak_f && last.map_or(true, |l| i - l >= 5) {
            hits.push((i * H) as f32 / SR16);
            last = Some(i);
        }
    }
    hits
}

/// The two networks and the front end.
pub struct Analyzer {
    emb: Model,
    head: Model,
    feats: Features,
    x: Vec<f32>,
    e: Vec<f32>,
    logits: Vec<f32>,
    pub load: Load,
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let emb = Model::from_bytes(EMBED_MODEL).expect("sorter embed model");
        let head = Model::from_bytes(HEAD_MODEL).expect("sorter head model");
        let load = Load::new(emb.macs() + head.macs());
        Analyzer { emb, head, feats: Features::new(), x: vec![0.0; MELS * FRAMES], e: vec![0.0; EMBED], logits: vec![0.0; N_CLASSES], load }
    }

    /// Analyses a sound (mono, 48 kHz).
    pub fn analyse(&mut self, audio: &[f32]) -> Analysis {
        let t = std::time::Instant::now();
        let mut d = Decimator::new();
        let mut x16 = Vec::with_capacity(audio.len() / 3 + 16);
        let n = audio.len().min((MAX_SECS * 48_000.0) as usize);
        d.push(&audio[..n], 48_000.0, &mut x16);
        let peak = x16.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        // The onset: the first sample within 34 dB of the peak.
        let onset = x16.iter().position(|v| v.abs() > peak * 0.02).unwrap_or(0);
        let start = onset as i64 - (PRE * HOP) as i64;
        let clip: Vec<f32> = (0..CLIP as i64).map(|i| {
            let j = start + i;
            if j < 0 || j as usize >= x16.len() {
                0.0
            } else {
                x16[j as usize]
            }
        }).collect();
        self.feats.compute(&clip, &mut self.x);
        self.emb.run(&self.x, &mut self.e);
        self.head.run(&self.e, &mut self.logits);
        let mut probs = self.logits.clone();
        neural::softmax(&mut probs);
        self.load.tick(t.elapsed());
        let norm = self.e.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
        let secs = audio.len() as f32 / 48_000.0;
        let hits = find_hits(&x16);
        let span = match (hits.first(), hits.last()) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        };
        let shown = (x16.len() as f32 / SR16).max(1e-3);
        let is_loop = shown >= 0.8 && hits.len() >= 4 && span >= 0.5 * shown;
        Analysis { class: neural::argmax(&probs), probs, emb: self.e.iter().map(|v| v / norm).collect(), secs, hits: hits.len() as u32, is_loop }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Relative to the library folder.
    pub path: String,
    pub bytes: u64,
    pub modified: u64,
    pub a: Analysis,
}

impl Entry {
    pub fn name(&self) -> String {
        Path::new(&self.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The two main directions of the embeddings (power iteration on the
/// covariance), and every entry's position along them scaled to 0..1.
pub fn project(entries: &[Entry]) -> Vec<(f32, f32)> {
    let n = entries.len();
    if n == 0 {
        return Vec::new();
    }
    let mut mean = vec![0.0f64; EMBED];
    for e in entries {
        for (m, v) in mean.iter_mut().zip(&e.a.emb) {
            *m += *v as f64 / n as f64;
        }
    }
    let mut cov = vec![vec![0.0f64; EMBED]; EMBED];
    for e in entries {
        let d: Vec<f64> = e.a.emb.iter().zip(&mean).map(|(v, m)| *v as f64 - m).collect();
        for i in 0..EMBED {
            for j in 0..EMBED {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let mut dirs: Vec<Vec<f64>> = Vec::new();
    for k in 0..2 {
        let mut v: Vec<f64> = (0..EMBED).map(|i| if i % 2 == k { 1.0 } else { 0.3 }).collect();
        for _ in 0..60 {
            let mut w: Vec<f64> = (0..EMBED).map(|i| (0..EMBED).map(|j| cov[i][j] * v[j]).sum()).collect();
            for d in &dirs {
                let p: f64 = w.iter().zip(d.iter()).map(|(a, b)| a * b).sum();
                for (wi, di) in w.iter_mut().zip(d.iter()) {
                    *wi -= p * di;
                }
            }
            let nrm = w.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
            v = w.iter().map(|x| x / nrm).collect();
        }
        dirs.push(v);
    }
    let pts: Vec<(f64, f64)> = entries
        .iter()
        .map(|e| {
            let d: Vec<f64> = e.a.emb.iter().zip(&mean).map(|(v, m)| *v as f64 - m).collect();
            (d.iter().zip(&dirs[0]).map(|(a, b)| a * b).sum(), d.iter().zip(&dirs[1]).map(|(a, b)| a * b).sum())
        })
        .collect();
    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    pts.iter().map(|p| (((p.0 - x0) / (x1 - x0).max(1e-9)) as f32, ((p.1 - y0) / (y1 - y0).max(1e-9)) as f32)).collect()
}

/// For each kit slot, the entry of that kind most like `seed`, each used
/// once; `variation` skips that many of the best matches.
pub fn build_kit(entries: &[Entry], seed: Option<&[f32]>, variation: usize) -> [Option<usize>; 16] {
    let mut used = vec![false; entries.len()];
    let mut kit = [None; 16];
    for (slot, &class) in KIT_SLOTS.iter().enumerate() {
        let mut cands: Vec<(usize, f32)> = entries
            .iter()
            .enumerate()
            // one-shots it's sure about: never a loop on the kick pad
            .filter(|(i, e)| e.a.class == class && !used[*i] && !e.a.is_loop && e.a.confident())
            .map(|(i, e)| (i, seed.map_or(e.a.probs[class], |s| cosine(s, &e.a.emb))))
            .collect();
        cands.sort_by(|a, b| b.1.total_cmp(&a.1));
        if let Some(&(i, _)) = cands.get(variation.min(cands.len().saturating_sub(1))) {
            used[i] = true;
            kit[slot] = Some(i);
        }
    }
    kit
}

pub struct Shared {
    pub entries: Mutex<Vec<Entry>>,
    /// Files waiting to be heard.
    queue: Mutex<Vec<PathBuf>>,
    pub total: AtomicUsize,
    pub done: AtomicUsize,
    pub busy: AtomicBool,
    load: Mutex<String>,
    root: PathBuf,
}

/// The worker: one file at a time.
struct Job {
    s: Arc<Shared>,
    an: Analyzer,
}

impl Job {
    fn step(&mut self) -> bool {
        let next = self.s.queue.lock().ok().and_then(|mut q| q.pop());
        let Some(path) = next else {
            self.s.busy.store(false, Ordering::Relaxed);
            return false;
        };
        self.s.busy.store(true, Ordering::Relaxed);
        if let Ok(audio) = crate::apps::chop::read_wav(&path) {
            if !audio.is_empty() {
                let a = self.an.analyse(&audio);
                let (bytes, modified) = file_stamp(&path);
                let rel = path.strip_prefix(&self.s.root).unwrap_or(&path).to_string_lossy().to_string();
                if let Ok(mut es) = self.s.entries.lock() {
                    es.retain(|e| e.path != rel);
                    es.push(Entry { path: rel, bytes, modified, a });
                }
            }
        }
        self.s.done.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut l) = self.s.load.lock() {
            *l = self.an.load.summary();
        }
        true
    }
}

fn file_stamp(p: &Path) -> (u64, u64) {
    let m = std::fs::metadata(p).ok();
    let bytes = m.as_ref().map_or(0, |m| m.len());
    let modified = m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    (bytes, modified)
}

/// Plays one sample at a time for auditioning; a new one cuts the old
/// with a 3 ms crossfade, so walking the map never clicks.
struct Player {
    slot: Arc<Mutex<Option<Arc<Vec<f32>>>>>,
    cur: Option<(Arc<Vec<f32>>, usize)>,
    old: Option<(Arc<Vec<f32>>, usize)>,
    fade: f32,
}

impl Player {
    fn read(v: &mut Option<(Arc<Vec<f32>>, usize)>) -> f32 {
        let Some((d, p)) = v.as_mut() else { return 0.0 };
        if *p >= d.len() {
            *v = None;
            return 0.0;
        }
        let y = d[*p];
        *p += 1;
        y
    }
}

impl Extra for Player {
    fn block(&mut self, _frames: usize, _sr: f32) {
        if let Ok(mut s) = self.slot.try_lock() {
            if let Some(n) = s.take() {
                self.old = self.cur.take();
                self.cur = Some((n, 0));
                self.fade = 0.0;
            }
        }
    }
    fn frame(&mut self, sr: f32) -> (f32, f32) {
        self.fade = (self.fade + 1.0 / (0.003 * sr)).min(1.0);
        let y = Player::read(&mut self.cur) * self.fade + Player::read(&mut self.old) * (1.0 - self.fade);
        if self.fade >= 1.0 {
            self.old = None;
        }
        // the kit halves its output: give it back
        (y * 2.0, y * 2.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Map,
    List,
    Kit,
}
const VIEWS: [View; 3] = [View::Map, View::List, View::Kit];

pub struct Sorter {
    sound: Sound,
    pub s: Arc<Shared>,
    worker: Option<Worker>,
    media: PathBuf,
    index: Option<PathBuf>,
    view: usize,
    /// The entry you're on (index into entries).
    cur: usize,
    class: usize,
    list_pos: usize,
    points: Vec<(f32, f32)>,
    points_for: usize,
    neighbours: Vec<usize>,
    kit: [Option<usize>; 16],
    variation: usize,
    kit_seed: Option<usize>,
    play: Arc<Mutex<Option<Arc<Vec<f32>>>>>,
    cache: HashMap<String, Arc<Vec<f32>>>,
    prev: [bool; 16],
    message: (String, u32),
    saved_at: usize,
    frame: u64,
    /// The map's zoom (1 = the whole library) easing toward `zoom_target`,
    /// and the centre of what's in view, in map coordinates (0..1).
    zoom: f32,
    zoom_target: f32,
    pan: (f32, f32),
    last_project: u64,
    list_cache: (usize, usize, Vec<usize>),
    map_px: Vec<Rgb565>,
    glow: Vec<[f32; 3]>,
}

impl Sorter {
    pub fn new(sound: Sound, media: PathBuf, index: Option<PathBuf>) -> Sorter {
        sound.set_reverb(0.0);
        sound.set_volume(1.0);
        let s = Arc::new(Shared { entries: Mutex::new(Vec::new()), queue: Mutex::new(Vec::new()), total: AtomicUsize::new(0), done: AtomicUsize::new(0), busy: AtomicBool::new(false), load: Mutex::new(String::new()), root: media.clone() });
        let mut so = Sorter { sound, s, worker: None, media, index, view: 0, cur: 0, class: 0, list_pos: 0, points: Vec::new(), points_for: usize::MAX, neighbours: Vec::new(), kit: [None; 16], variation: 0, kit_seed: None, play: Arc::new(Mutex::new(None)), cache: HashMap::new(), prev: [false; 16], message: (String::new(), 0), saved_at: 0, frame: 0, zoom: 1.0, zoom_target: 1.0, pan: (0.5, 0.5), last_project: 0, list_cache: (usize::MAX, 0, Vec::new()), map_px: vec![Rgb565::new(0, 0, 0); MAP_W * MAP_H], glow: vec![[0.0; 3]; MAP_W * MAP_H] };
        so.load_index();
        so
    }

    fn flash(&mut self, m: impl Into<String>) {
        self.message = (m.into(), 180);
    }

    fn load_index(&mut self) {
        let Some(p) = self.index.as_ref() else { return };
        let Ok(j) = std::fs::read_to_string(p) else { return };
        if let Ok(es) = serde_json::from_str::<Vec<Entry>>(&j) {
            let es: Vec<Entry> = es.into_iter().filter(|e| e.a.emb.len() == EMBED && e.a.probs.len() == N_CLASSES).collect();
            self.saved_at = es.len();
            *self.s.entries.lock().unwrap() = es;
        }
    }

    fn save_index(&mut self) {
        let Some(p) = self.index.clone() else { return };
        let es = self.s.entries.lock().unwrap().clone();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(j) = serde_json::to_string(&es) {
            let _ = std::fs::write(&p, j);
        }
        self.saved_at = es.len();
    }

    /// Finds new or changed WAVs and queues them; forgets removed ones.
    pub fn rescan(&mut self) {
        let mut files = Vec::new();
        scan(&self.media, &mut files, 0);
        let mut es = self.s.entries.lock().unwrap();
        let known: HashMap<String, (u64, u64)> = es.iter().map(|e| (e.path.clone(), (e.bytes, e.modified))).collect();
        let present: std::collections::HashSet<String> = files.iter().map(|p| p.strip_prefix(&self.media).unwrap_or(p).to_string_lossy().to_string()).collect();
        es.retain(|e| present.contains(&e.path));
        drop(es);
        let todo: Vec<PathBuf> = files
            .into_iter()
            .filter(|p| {
                let rel = p.strip_prefix(&self.media).unwrap_or(p).to_string_lossy().to_string();
                known.get(&rel) != Some(&file_stamp(p))
            })
            .collect();
        self.s.total.store(todo.len(), Ordering::Relaxed);
        self.s.done.store(0, Ordering::Relaxed);
        if !todo.is_empty() {
            self.s.busy.store(true, Ordering::Relaxed);
        }
        *self.s.queue.lock().unwrap() = todo;
        self.points_for = usize::MAX;
    }

    /// For tests: analyses everything queued, here and now.
    #[cfg(test)]
    fn analyse_all(&mut self) {
        let mut job = Job { s: Arc::clone(&self.s), an: Analyzer::new() };
        while job.step() {}
    }

    fn entries(&self) -> Vec<Entry> {
        self.s.entries.lock().map(|e| e.clone()).unwrap_or_default()
    }

    /// Lays the library out again when it has changed. While the worker is
    /// still listening this is throttled: with thousands of sounds the
    /// projection is real work, and it shouldn't run on every new file.
    fn refresh(&mut self) {
        let s = Arc::clone(&self.s);
        let es = s.entries.lock().unwrap_or_else(|p| p.into_inner());
        let busy = self.s.busy.load(Ordering::Relaxed);
        let due = self.points_for == usize::MAX || self.points.is_empty() || !busy || self.frame.saturating_sub(self.last_project) >= 150;
        if es.len() != self.points_for && due {
            self.points = project(&es);
            self.points_for = es.len();
            self.last_project = self.frame;
            if self.cur >= es.len() {
                self.cur = 0;
            }
            self.find_neighbours(&es);
        }
    }

    fn find_neighbours(&mut self, es: &[Entry]) {
        self.neighbours.clear();
        let Some(c) = es.get(self.cur) else { return };
        let mut by: Vec<(usize, f32)> = es.iter().enumerate().filter(|(i, _)| *i != self.cur).map(|(i, e)| (i, cosine(&c.a.emb, &e.a.emb))).collect();
        by.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.neighbours = by.into_iter().take(16).map(|(i, _)| i).collect();
    }

    fn select(&mut self, i: usize) {
        self.cur = i;
        let es = self.entries();
        self.find_neighbours(&es);
    }

    fn audition(&mut self, i: usize) {
        let es = self.entries();
        let Some(e) = es.get(i) else { return };
        let key = e.path.clone();
        let data = if let Some(d) = self.cache.get(&key) {
            Arc::clone(d)
        } else {
            let Ok(mut a) = crate::apps::chop::read_wav(&self.media.join(&e.path)) else {
                self.flash("couldn't read it");
                return;
            };
            a.truncate((MAX_SECS * 48_000.0) as usize);
            let d = Arc::new(a);
            if self.cache.len() > 64 {
                self.cache.clear();
            }
            self.cache.insert(key, Arc::clone(&d));
            d
        };
        *self.play.lock().unwrap() = Some(data);
    }

    /// The nearest entry from the current one in a direction on the map.
    fn walk(&mut self, dx: f32, dy: f32) {
        let Some(&(x, y)) = self.points.get(self.cur) else { return };
        let mut best = None;
        let mut best_d = f32::MAX;
        for (i, &(px, py)) in self.points.iter().enumerate() {
            if i == self.cur {
                continue;
            }
            let (vx, vy) = (px - x, py - y);
            let along = vx * dx + vy * dy;
            if along <= 1e-6 {
                continue;
            }
            let across = (vx * dy - vy * dx).abs();
            // prefer straight ahead: sideways distance counts double
            let d = along + 2.0 * across;
            if d < best_d {
                best_d = d;
                best = Some(i);
            }
        }
        if let Some(i) = best {
            self.select(i);
            self.audition(i);
        }
    }

    /// One kind's one-shots, most typical first; `LOOPS` lists the loops.
    fn class_list(es: &[Entry], class: usize) -> Vec<usize> {
        let mut v: Vec<(usize, f32)> = if class == LOOPS {
            es.iter().enumerate().filter(|(_, e)| e.a.is_loop).map(|(i, e)| (i, e.a.hits as f32)).collect()
        } else {
            es.iter().enumerate().filter(|(_, e)| e.a.class == class && !e.a.is_loop).map(|(i, e)| (i, e.a.probs[class])).collect()
        };
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v.into_iter().map(|(i, _)| i).collect()
    }

    /// The current kind's list, sorted again only when the kind or the
    /// library changes (it's re-sorted per frame otherwise: costly at 10,000).
    fn list_with(&mut self, es: &[Entry]) -> Vec<usize> {
        if self.list_cache.0 != self.class || self.list_cache.1 != es.len() {
            self.list_cache = (self.class, es.len(), Self::class_list(es, self.class));
        }
        self.list_cache.2.clone()
    }

    fn list(&mut self) -> Vec<usize> {
        let s = Arc::clone(&self.s);
        let es = s.entries.lock().unwrap_or_else(|p| p.into_inner());
        self.list_with(&es)
    }

    fn make_kit(&mut self) {
        let es = self.entries();
        let seed = es.get(self.cur).map(|e| e.a.emb.clone());
        self.kit = build_kit(&es, seed.as_deref(), self.variation);
        self.kit_seed = Some(self.cur);
        let filled = self.kit.iter().filter(|k| k.is_some()).count();
        self.flash(format!("kit: {filled} of 16 pads filled"));
    }

    /// Copies the kit's files into media/Kits/Sorter Kit N/.
    pub fn export_kit(&mut self) -> Option<PathBuf> {
        let es = self.entries();
        if self.kit.iter().all(|k| k.is_none()) {
            self.flash("build a kit first");
            return None;
        }
        let kits = self.media.join("Kits");
        let mut n = 1;
        while kits.join(format!("Sorter Kit {n}")).exists() {
            n += 1;
        }
        let dir = kits.join(format!("Sorter Kit {n}"));
        if std::fs::create_dir_all(&dir).is_err() {
            self.flash("couldn't make the folder");
            return None;
        }
        let mut copied = 0;
        for (slot, k) in self.kit.iter().enumerate() {
            let Some(e) = k.and_then(|i| es.get(i)) else { continue };
            let class = CLASSES[KIT_SLOTS[slot]];
            let name = format!("{:02} {} - {}.wav", slot + 1, class, e.name());
            if std::fs::copy(self.media.join(&e.path), dir.join(name)).is_ok() {
                copied += 1;
            }
        }
        self.flash(format!("exported {copied} sounds to Kits/Sorter Kit {n}"));
        // the kit's own files will be found next scan, under its folder
        Some(dir)
    }
}

impl Sorter {
    /// Eases the zoom (on a log scale, so it feels even) and keeps the view
    /// on the sound you're on, never past the edge of the map.
    fn ease_view(&mut self) {
        self.zoom = (self.zoom.ln() + (self.zoom_target.ln() - self.zoom.ln()) * 0.25).exp();
        if (self.zoom - self.zoom_target).abs() < 0.002 {
            self.zoom = self.zoom_target;
        }
        let want = if self.zoom_target > 1.0 { self.points.get(self.cur).copied().unwrap_or((0.5, 0.5)) } else { (0.5, 0.5) };
        let half = 0.5 / self.zoom;
        let goal = (want.0.clamp(half, 1.0 - half), want.1.clamp(half, 1.0 - half));
        self.pan.0 += (goal.0 - self.pan.0) * 0.3;
        self.pan.1 += (goal.1 - self.pan.1) * 0.3;
    }

    /// A sound's on-screen size: longer sounds are bigger, on a log scale.
    fn orb_size(secs: f32) -> f32 {
        (0.6 + 0.35 * (1.0 + secs * 4.0).ln()).clamp(0.6, 2.1)
    }

    /// Where the map point `p` lands in the map's own pixels.
    fn to_map(&self, p: (f32, f32)) -> (f32, f32) {
        (((p.0 - self.pan.0) * self.zoom + 0.5) * MAP_W as f32, ((self.pan.1 - p.1) * self.zoom + 0.5) * MAP_H as f32)
    }

    /// The map: every sound a soft orb (a ring for a loop), lit additively
    /// so a crowded neighbourhood glows rather than clogging, with the
    /// pad neighbours tied to the sound you're on by thin lines.
    fn draw_map(&mut self, fb: &mut FrameBuffer, es: &[Entry], panel: Rgb565, dim: Rgb565) {
        let (ox, oy) = (MAP_X, MAP_Y);
        kit::round_rect(fb, ox - 4, oy - 4, MAP_W as i32 + 8, MAP_H as i32 + 8, 12, panel);
        let z = self.zoom;
        let lz = z.ln();
        let t = (lz / 8f32.ln()).clamp(0.0, 1.0);
        let gain = 0.5 + 0.7 * t;
        self.glow.iter_mut().for_each(|g| *g = [0.0; 3]);
        let mut in_view = 0usize;
        let pad = 10.0;
        for (e, &p) in es.iter().zip(self.points.iter()) {
            let (x, y) = self.to_map(p);
            if x < -pad || y < -pad || x > MAP_W as f32 + pad || y > MAP_H as f32 + pad {
                continue;
            }
            in_view += 1;
            let sf = Self::orb_size(e.a.secs);
            let r = ((1.1 * sf) * (1.0 + 0.9 * lz)).clamp(1.0, 12.0);
            let c = class_rgb(e.a.class);
            if e.a.is_loop {
                splat_ring(&mut self.glow, x, y, r.max(2.2), c, gain);
            } else {
                splat(&mut self.glow, x, y, r, c, gain);
                if z >= 2.0 {
                    splat(&mut self.glow, x, y, (r * 0.4).max(0.9), [1.0, 1.0, 1.0], 0.6);
                }
            }
        }
        // light to colour, with a faint grid that follows the zoom
        let step = if z < 3.0 { 1.0 / 8.0 } else if z < 6.0 { 1.0 / 16.0 } else if z < 12.0 { 1.0 / 32.0 } else { 1.0 / 64.0 };
        let (vx0, vx1) = (self.pan.0 - 0.5 / z, self.pan.0 + 0.5 / z);
        let (vy0, vy1) = (self.pan.1 - 0.5 / z, self.pan.1 + 0.5 / z);
        let mut gx = Vec::new();
        let mut k = (vx0 / step).ceil();
        while k * step <= vx1 && gx.len() < 80 {
            gx.push(self.to_map((k * step, 0.0)).0.round() as i32);
            k += 1.0;
        }
        let mut gy = Vec::new();
        let mut k = (vy0 / step).ceil();
        while k * step <= vy1 && gy.len() < 80 {
            gy.push(self.to_map((0.0, k * step)).1.round() as i32);
            k += 1.0;
        }
        let base = [panel.r() as f32 / 31.0 * 0.75, panel.g() as f32 / 63.0 * 0.75, panel.b() as f32 / 31.0 * 0.78];
        for y in 0..MAP_H {
            let on_row = gy.contains(&(y as i32));
            for x in 0..MAP_W {
                let g = self.glow[y * MAP_W + x];
                let grid = if on_row || gx.contains(&(x as i32)) { 0.035 } else { 0.0 };
                let ch = |i: usize| (base[i] + grid + (1.0 - (-g[i] * 1.5).exp()) * (1.0 - base[i])).clamp(0.0, 1.0);
                self.map_px[y * MAP_W + x] = Rgb565::new((ch(0) * 31.0) as u8, (ch(1) * 63.0) as u8, (ch(2) * 31.0) as u8);
            }
        }
        if z > 1.05 {
            self.draw_minimap(es);
        }
        let area = Rectangle::new(Point::new(ox, oy), Size::new(MAP_W as u32, MAP_H as u32));
        fb.fill_contiguous(&area, self.map_px.iter().copied()).ok();

        // what you're on, and the sounds the pads play
        let inside = |x: f32, y: f32, m: f32| x >= m && y >= m && x <= MAP_W as f32 - m && y <= MAP_H as f32 - m;
        if let Some(&p) = self.points.get(self.cur) {
            let (cx, cy) = self.to_map(p);
            for (n, &i) in self.neighbours.iter().enumerate() {
                let (Some(e), Some(&q)) = (es.get(i), self.points.get(i)) else { continue };
                let (nx, ny) = self.to_map(q);
                if let Some((a, b)) = clip_line((cx, cy), (nx, ny), MAP_W as f32 - 1.0, MAP_H as f32 - 1.0) {
                    kit::line(fb, ox + a.0 as i32, oy + a.1 as i32, ox + b.0 as i32, oy + b.1 as i32, 1, kit::blend(panel, kit::WHITE, 0.2));
                }
                if inside(nx, ny, 6.0) {
                    let (sx, sy) = (ox + nx as i32, oy + ny as i32);
                    let col = class_color(e.a.class);
                    let r = (2.0 + 0.5 * lz).min(6.0) as i32;
                    if e.a.is_loop {
                        kit::ring(fb, sx, sy, r + 1, 2, col);
                    } else {
                        kit::circle(fb, sx, sy, r, col);
                    }
                    kit::ring(fb, sx, sy, r + 2, 1, kit::WHITE);
                    if z >= 2.5 {
                        let label = format!("{}", n + 1);
                        kit::text(fb, &label, sx + r + 4, sy - 6, Size2::Small, kit::WHITE, -1);
                        if z >= 8.0 {
                            let nm: String = e.name().chars().take(14).collect();
                            kit::text(fb, &nm, sx + r + 4 + kit::text_width(&label, Size2::Small) + 4, sy - 6, Size2::Small, dim, -1);
                        }
                    }
                }
            }
            if inside(cx, cy, 2.0) {
                let (sx, sy) = (ox + cx as i32, oy + cy as i32);
                let pulse = ((self.frame as f32 * 0.12).sin() * 1.2) as i32;
                let r = 9 + pulse;
                kit::ring(fb, sx, sy, r, 1, kit::WHITE);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    kit::line(fb, sx + dx * (r + 2), sy + dy * (r + 2), sx + dx * (r + 6), sy + dy * (r + 6), 1, kit::WHITE);
                }
                if let Some(e) = es.get(self.cur) {
                    if z >= 1.5 {
                        let nm: String = e.name().chars().take(26).collect();
                        let w = kit::text_width(&nm, Size2::Small);
                        let lx = (sx + 14).min(ox + MAP_W as i32 - w - 4).max(ox + 4);
                        kit::round_rect(fb, lx - 3, sy + 12, w + 6, 15, 7, kit::rgb(8, 10, 14));
                        kit::text(fb, &nm, lx, sy + 14, Size2::Small, kit::WHITE, -1);
                    }
                }
            }
        }
        // zoom and what's in view, bottom left
        let note = if z < 1.05 { "K1: zoom in".to_string() } else { format!("{:.1}x   {} in view", z, thousands(in_view)) };
        let w = kit::text_width(&note, Size2::Small) + 14;
        kit::round_rect(fb, ox + 8, oy + MAP_H as i32 - 24, w, 17, 8, kit::rgb(8, 10, 14));
        kit::text(fb, &note, ox + 15, oy + MAP_H as i32 - 21, Size2::Small, if z < 1.05 { dim } else { kit::WHITE }, -1);
        if es.is_empty() {
            kit::paragraph(fb, "No sounds yet. Put WAV files in the media folder (any subfolders) and they'll be heard and sorted here.", ox + 90, oy + 100, MAP_W as i32 - 180, Size2::Small, dim);
        }
    }

    /// The whole library in a corner, with a frame on the part in view.
    fn draw_minimap(&mut self, es: &[Entry]) {
        let (mw, mh) = (96usize, 62usize);
        let (mx, my) = (MAP_W - mw - 8, 8usize);
        let dark = |c: Rgb565, t: f32| Rgb565::new((c.r() as f32 * t) as u8, (c.g() as f32 * t) as u8, (c.b() as f32 * t) as u8);
        for y in my..my + mh {
            for x in mx..mx + mw {
                let c = self.map_px[y * MAP_W + x];
                self.map_px[y * MAP_W + x] = dark(c, 0.28);
            }
        }
        let n = self.points.len().min(es.len());
        let skip = (n / 2500).max(1);
        for i in (0..n).step_by(skip) {
            let (px, py) = self.points[i];
            let x = mx + 2 + (px * (mw - 5) as f32) as usize;
            let y = my + 2 + ((1.0 - py) * (mh - 5) as f32) as usize;
            let (r, g, b) = COLORS[es[i].a.class % N_CLASSES];
            self.map_px[y * MAP_W + x] = kit::rgb(r / 3 + 30, g / 3 + 30, b / 3 + 30);
        }
        let to_mini = |p: (f32, f32)| (mx as f32 + 2.0 + p.0 * (mw - 5) as f32, my as f32 + 2.0 + (1.0 - p.1) * (mh - 5) as f32);
        let (a, b) = (to_mini((self.pan.0 - 0.5 / self.zoom, self.pan.1 + 0.5 / self.zoom)), to_mini((self.pan.0 + 0.5 / self.zoom, self.pan.1 - 0.5 / self.zoom)));
        let (x0, x1, y0, y1) = (a.0.max(mx as f32) as usize, (b.0.min((mx + mw - 1) as f32)) as usize, a.1.max(my as f32) as usize, (b.1.min((my + mh - 1) as f32)) as usize);
        for x in x0..=x1.max(x0) {
            self.map_px[y0 * MAP_W + x] = kit::WHITE;
            self.map_px[y1.max(y0) * MAP_W + x] = kit::WHITE;
        }
        for y in y0..=y1.max(y0) {
            self.map_px[y * MAP_W + x0] = kit::WHITE;
            self.map_px[y * MAP_W + x1.max(x0)] = kit::WHITE;
        }
    }

    /// Kinds down the side, a round swatch each, loops last with its ring.
    fn draw_legend(&mut self, fb: &mut FrameBuffer, es: &[Entry], panel: Rgb565, dim: Rgb565) {
        let (lx, lw) = (LEGEND_X, 640 - LEGEND_X - 8);
        kit::round_rect(fb, lx, MAP_Y - 4, lw, MAP_H as i32 + 8, 12, panel);
        for c in 0..=N_CLASSES {
            let y = MAP_Y + 4 + c as i32 * 17;
            let (count, name) = if c == LOOPS {
                (es.iter().filter(|e| e.a.is_loop).count(), "loops")
            } else {
                (es.iter().filter(|e| e.a.class == c && !e.a.is_loop).count(), CLASSES[c])
            };
            let col = if c == LOOPS { kit::rgb(200, 200, 210) } else { class_color(c) };
            let live = count > 0;
            if c == LOOPS {
                kit::ring(fb, lx + 13, y + 6, 4, 2, if live { col } else { dim });
            } else {
                kit::circle(fb, lx + 13, y + 6, 4, if live { col } else { kit::blend(panel, col, 0.35) });
            }
            kit::text(fb, name, lx + 24, y, Size2::Small, if live { kit::WHITE } else { dim }, -1);
            kit::text(fb, &thousands(count), lx + lw - 8, y, Size2::Small, dim, 1);
        }
    }

}

fn scan(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            scan(&p, out, depth + 1);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            out.push(p);
        }
    }
}

impl App for Sorter {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        let es = self.s.entries.lock().unwrap_or_else(|p| p.into_inner());
        vec![
            ("View".into(), format!("{:?}", VIEWS[self.view]), false),
            ("Sounds".into(), format!("{}", es.len()), false),
            ("Zoom".into(), format!("{:.1}x", self.zoom), false),
            ("Current".into(), es.get(self.cur).map(|e| e.name()).unwrap_or_default(), false),
        ]
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match VIEWS[self.view] {
            View::Map => "MAP",
            View::List => "LIST",
            View::Kit => "KIT",
        })
    }
    fn toggle_grid_mode(&mut self) {
        self.view = (self.view + 1) % VIEWS.len();
    }
    fn on_enter(&mut self) {
        self.rescan();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        std::array::from_fn(|p| match VIEWS[self.view] {
            View::Kit => {
                if self.kit[p].is_some() {
                    PadColor::Green
                } else {
                    PadColor::Off
                }
            }
            _ => {
                if p < self.neighbours.len() {
                    PadColor::Yellow
                } else {
                    PadColor::Off
                }
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        if self.message.1 > 0 {
            self.message.1 -= 1;
        }
        // The worker starts the first time there's something to hear.
        if self.worker.is_none() && self.s.busy.load(Ordering::Relaxed) && !cfg!(test) {
            let mut job = Job { s: Arc::clone(&self.s), an: Analyzer::new() };
            self.worker = Some(Worker::spawn("sorter-npu", move || {
                // a few files per wake, so a big library doesn't crawl
                for _ in 0..4 {
                    if !job.step() {
                        break;
                    }
                }
            }));
        }
        if self.frame % 30 == 0 {
            self.refresh();
            let n = self.s.entries.lock().map(|e| e.len()).unwrap_or(0);
            if !self.s.busy.load(Ordering::Relaxed) && n != self.saved_at {
                self.save_index();
            }
        }
        self.ease_view();
        let pressed: Vec<usize> = (0..16).filter(|&p| input.grid[p] && !self.prev[p]).collect();
        self.prev = input.grid;
        match VIEWS[self.view] {
            View::Map => {
                if input.knob1 != 0 {
                    self.zoom_target = (self.zoom_target * 1.5f32.powi(input.knob1.clamp(-4, 4))).clamp(1.0, MAX_ZOOM);
                }
                if input.navigation_steps != 0 {
                    self.walk(0.0, -input.navigation_steps.signum() as f32);
                }
                if input.knob2 != 0 {
                    self.walk(input.knob2.signum() as f32, 0.0);
                }
                if input.knob1_press {
                    let c = self.cur;
                    self.audition(c);
                }
                for p in pressed {
                    if let Some(&i) = self.neighbours.get(p) {
                        self.audition(i);
                        // and walk there, so the pads re-centre on it
                        self.select(i);
                    }
                }
            }
            View::List => {
                if input.knob2 != 0 {
                    self.class = (self.class as i32 + input.knob2.signum()).rem_euclid(N_CLASSES as i32 + 1) as usize;
                    self.list_pos = 0;
                }
                let list = self.list();
                if input.navigation_steps != 0 && !list.is_empty() {
                    self.list_pos = (self.list_pos as i32 + input.navigation_steps).clamp(0, list.len() as i32 - 1) as usize;
                    let i = list[self.list_pos];
                    self.select(i);
                    self.audition(i);
                }
                if input.knob1_press {
                    if let Some(&i) = list.get(self.list_pos) {
                        self.select(i);
                        self.audition(i);
                    }
                }
                for p in pressed {
                    if let Some(&i) = list.get(p) {
                        self.list_pos = p;
                        self.select(i);
                        self.audition(i);
                    }
                }
            }
            View::Kit => {
                if input.knob1_press {
                    self.make_kit();
                }
                if input.knob2 != 0 {
                    self.variation = (self.variation as i32 + input.knob2.signum()).clamp(0, 9) as usize;
                    self.make_kit();
                }
                if input.navigation_steps != 0 {
                    // down exports
                    if input.navigation_steps > 0 {
                        self.export_kit();
                    }
                }
                for p in pressed {
                    if let Some(i) = self.kit[p] {
                        self.audition(i);
                    }
                }
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::rgb(12, 15, 21);
        let panel = kit::rgb(24, 29, 39);
        let dim = kit::rgb(125, 135, 150);
        kit::round_rect(fb, 0, 0, 640, 360, 0, bg);
        // Held for the whole frame: with thousands of sounds, copying the
        // library 60 times a second is the one thing that would stutter.
        let shared = Arc::clone(&self.s);
        let guard = shared.entries.lock().unwrap_or_else(|p| p.into_inner());
        let es: &[Entry] = &guard;
        if self.points.is_empty() && !es.is_empty() {
            self.points = project(es);
            self.points_for = es.len();
            self.find_neighbours(es);
        }

        // title, the three views as tabs, and what the ear is doing
        kit::text(fb, NAME, 14, 9, Size2::Medium, kit::WHITE, -1);
        let mut tx = 86;
        for (i, name) in ["MAP", "LIST", "KIT"].iter().enumerate() {
            let w = kit::text_width(name, Size2::Small) + 16;
            let on = i == self.view;
            kit::round_rect(fb, tx, 8, w, 18, 9, if on { kit::blend(panel, kit::WHITE, 0.2) } else { bg });
            kit::text(fb, name, tx + w / 2, 11, Size2::Small, if on { kit::WHITE } else { dim }, 0);
            tx += w + 4;
        }
        let busy = self.s.busy.load(Ordering::Relaxed);
        let status = if busy {
            format!("listening {}/{}", self.s.done.load(Ordering::Relaxed), self.s.total.load(Ordering::Relaxed))
        } else {
            format!("{} sounds", thousands(es.len()))
        };
        kit::text(fb, &status, 628, 11, Size2::Small, dim, 1);
        if busy {
            let t = self.s.total.load(Ordering::Relaxed).max(1);
            let d = self.s.done.load(Ordering::Relaxed);
            let sw = kit::text_width(&status, Size2::Small);
            let (bx, bw) = (628 - sw - 14 - 120, 120);
            kit::round_rect(fb, bx, 14, bw, 4, 2, panel);
            kit::round_rect(fb, bx, 14, ((bw * d as i32) / t as i32).max(3), 4, 2, kit::rgb(120, 220, 160));
        }
        let cur = es.get(self.cur);
        match VIEWS[self.view] {
            View::Map => {
                self.draw_map(fb, es, panel, dim);
                self.draw_legend(fb, es, panel, dim);
            }
            View::List => {
                let head_c = if self.class == LOOPS { kit::rgb(200, 200, 210) } else { class_color(self.class) };
                kit::round_rect(fb, 8, 34, 624, 28, 14, kit::blend(panel, head_c, 0.22));
                kit::text(fb, "<", 24, 41, Size2::Medium, dim, -1);
                kit::text(fb, ">", 616, 41, Size2::Medium, dim, 1);
                kit::circle(fb, 320 - kit::text_width(kind_name(self.class), Size2::Medium) / 2 - 14, 48, 5, head_c);
                kit::text(fb, kind_name(self.class), 320, 41, Size2::Medium, kit::WHITE, 0);
                let list = self.list_with(es);
                let first = self.list_pos.saturating_sub(4);
                for (row, &i) in list.iter().enumerate().skip(first).take(10) {
                    let e = &es[i];
                    let y = 70 + (row - first) as i32 * 24;
                    let sel = row == self.list_pos;
                    kit::round_rect(fb, 8, y, 624, 22, 11, if sel { kit::blend(panel, kit::WHITE, 0.14) } else { panel });
                    let c = if self.class == LOOPS { class_color(e.a.class) } else { class_color(self.class) };
                    if e.a.is_loop {
                        kit::ring(fb, 24, y + 11, 4, 2, class_color(e.a.class));
                    } else {
                        kit::circle(fb, 24, y + 11, 4, class_color(e.a.class));
                    }
                    let name: String = e.name().chars().take(46).collect();
                    kit::text(fb, &name, 38, y + 5, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
                    let info = if self.class == LOOPS {
                        format!("{} hits, starts {}  {:.2}s", e.a.hits, CLASSES[e.a.class], e.a.secs)
                    } else {
                        format!("{:.0}%  {:.2}s", e.a.probs[self.class] * 100.0, e.a.secs)
                    };
                    kit::text(fb, &info, 618, y + 5, Size2::Small, c, 1);
                }
                if list.is_empty() {
                    kit::text(fb, "none of this kind", 320, 150, Size2::Small, dim, 0);
                } else {
                    kit::text(fb, &format!("{} of {}", self.list_pos + 1, thousands(list.len())), 320, 314, Size2::Small, dim, 0);
                }
            }
            View::Kit => {
                for p in 0..16 {
                    let (cx, cy) = (p % 4, p / 4);
                    let (x, y) = (8 + cx as i32 * 104, 38 + cy as i32 * 68);
                    let class = KIT_SLOTS[p];
                    let col = class_color(class);
                    let filled = self.kit[p].is_some();
                    kit::round_rect(fb, x, y, 98, 62, 12, kit::blend(panel, col, if filled { 0.28 } else { 0.07 }));
                    kit::circle(fb, x + 12, y + 12, 4, if filled { col } else { kit::blend(panel, col, 0.5) });
                    kit::text(fb, CLASSES[class], x + 22, y + 6, Size2::Small, col, -1);
                    if let Some(e) = self.kit[p].and_then(|i| es.get(i)) {
                        let name: String = e.name().chars().take(15).collect();
                        kit::text(fb, &name, x + 8, y + 28, Size2::Small, kit::WHITE, -1);
                        kit::text(fb, &format!("{:.2}s", e.a.secs), x + 8, y + 44, Size2::Small, dim, -1);
                    } else {
                        kit::text(fb, "-", x + 8, y + 28, Size2::Small, dim, -1);
                    }
                }
                let seed = self.kit_seed.and_then(|i| es.get(i)).map(|e| e.name()).unwrap_or_else(|| "(none yet)".into());
                kit::text(fb, "built around", 436, 42, Size2::Small, dim, -1);
                let s: String = seed.chars().take(30).collect();
                kit::text(fb, &s, 436, 60, Size2::Small, kit::WHITE, -1);
                kit::text(fb, &format!("variation {}", self.variation + 1), 436, 88, Size2::Small, dim, -1);
                kit::paragraph(fb, "SELECT builds a kit around the sound you're on (pick it in MAP or LIST). Left/right: other takes. Down: export to media/Kits.", 436, 118, 192, Size2::Small, dim);
            }
        }
        if let Some(e) = cur {
            let name: String = e.name().chars().take(40).collect();
            kit::round_rect(fb, 8, 318, 624, 20, 10, panel);
            if e.a.is_loop {
                kit::ring(fb, 22, 328, 4, 2, class_color(e.a.class));
            } else {
                kit::circle(fb, 22, 328, 4, class_color(e.a.class));
            }
            kit::text(fb, &name, 34, 322, Size2::Small, kit::WHITE, -1);
            let what = if e.a.is_loop {
                format!("loop, {} hits, starts {}", e.a.hits, CLASSES[e.a.class])
            } else if e.a.confident() {
                format!("{} {:.0}%", CLASSES[e.a.class], e.a.probs[e.a.class] * 100.0)
            } else {
                format!("{}? {:.0}%", CLASSES[e.a.class], e.a.probs[e.a.class] * 100.0)
            };
            kit::text(fb, &format!("{what}   {:.2}s", e.a.secs), 622, 322, Size2::Small, dim, 1);
        }
        if self.message.1 > 0 {
            kit::round_rect(fb, 140, 288, 360, 24, 12, kit::rgb(110, 90, 40));
            kit::text(fb, &self.message.0, 320, 294, Size2::Small, kit::WHITE, 0);
        }
        let help = match VIEWS[self.view] {
            View::Map => "F2 view   K1 zoom   d-pad walk   pads: 16 most alike   SELECT play",
            View::List => "F2 view   left/right: kind   up/down: sound   pads: play",
            View::Kit => "F2 view   SELECT: build   left/right: variation   down: export",
        };
        kit::footer(fb, help, panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let p = Player { slot: Arc::clone(&self.play), cur: None, old: None, fade: 1.0 };
        Some(self.sound.processor(None, Some(Box::new(p))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let media = std::env::var_os("PORTAMAX_MEDIA_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("media"));
    let index = (!cfg!(test)).then(|| root.join("saves/sorter/index.json"));
    Box::new(Sorter::new(Sound::new(NAME, &modbus, &mixer, &bus), media, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::render;
    use std::time::Instant;
    use crate::apps::kids_kit::{Drum, Note, Tone};

    #[test]
    fn the_models_match_python_bit_for_bit() {
        crate::apps::neural::tests::check_vectors(EMBED_MODEL, include_str!("../../assets/npu/sorter_embed.test.json"));
        crate::apps::neural::tests::check_vectors(HEAD_MODEL, include_str!("../../assets/npu/sorter_head.test.json"));
    }

    #[test]
    fn features_match_python() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../assets/npu/sorter.features.json")).unwrap();
        let clip: Vec<f32> = v["clip"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let want: Vec<f32> = v["features"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let mut out = vec![0.0; MELS * FRAMES];
        Features::new().compute(&clip, &mut out);
        let worst = out.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
    }

    /// A sound from the device's own synths, which the network never
    /// heard in training (left channel, 48 kHz, 0.8 s).
    fn kit_sound(f: impl FnOnce(&Sound)) -> Vec<f32> {
        let snd = Sound::detached();
        snd.set_reverb(0.0);
        let mut p = snd.processor(None, None);
        f(&snd);
        let st = render(&mut p, 75);
        st.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn it_names_drums_it_never_heard_in_training() {
        let mut an = Analyzer::new();
        let cases: Vec<(&str, Vec<f32>, &[&str])> = vec![
            ("kick", kit_sound(|s| s.drum(Drum::Kick, 1.0)), &["kick"]),
            ("snare", kit_sound(|s| s.drum(Drum::Snare, 1.0)), &["snare", "clap"]),
            ("hat", kit_sound(|s| s.drum(Drum::Hat, 1.0)), &["closed hat", "shaker"]),
            ("open hat", kit_sound(|s| s.drum(Drum::OpenHat, 1.0)), &["open hat", "cymbal"]),
            ("clap", kit_sound(|s| s.drum(Drum::Clap, 1.0)), &["clap", "snare"]),
            ("rim", kit_sound(|s| s.drum(Drum::Rim, 1.0)), &["rim", "hand perc"]),
            ("cowbell", kit_sound(|s| s.drum(Drum::Cowbell, 1.0)), &["metal"]),
            ("woodblock", kit_sound(|s| s.drum(Drum::Woodblock, 1.0)), &["hand perc", "rim"]),
            ("high tom", kit_sound(|s| s.drum(Drum::HiTom, 1.0)), &["tom", "hand perc"]),
            ("low tom", kit_sound(|s| s.drum(Drum::LowTom, 1.0)), &["tom", "kick"]),
            ("shaker", kit_sound(|s| s.drum(Drum::Shaker, 1.0)), &["shaker", "closed hat"]),
            ("bass note", kit_sound(|s| s.play(Note::new(Tone::Bass, 33.0).len(0.7))), &["bass"]),
            ("organ chord", kit_sound(|s| {
                for n in [60.0, 64.0, 67.0] {
                    s.play(Note::new(Tone::Organ, n).len(0.7));
                }
            }), &["tonal", "texture"]),
            ("harp", kit_sound(|s| s.play(Note::new(Tone::Pluck, 67.0).len(0.5))), &["tonal"]),
        ];
        let mut right = 0;
        let mut report = String::new();
        for (name, audio, ok) in &cases {
            let a = an.analyse(audio);
            let got = CLASSES[a.class];
            let hit = ok.contains(&got);
            right += hit as usize;
            report += &format!("\n  {name:12} -> {got:10} {:3.0}% {}", a.probs[a.class] * 100.0, if hit { "" } else { "  <-- wrong" });
        }
        println!("{report}");
        // Exact names on unseen generators: most, not all.
        assert!(right * 100 >= cases.len() * 75, "{right}/{} right:{report}", cases.len());
    }

    #[test]
    fn similar_sounds_are_close() {
        let mut an = Analyzer::new();
        let k1 = an.analyse(&kit_sound(|s| s.drum(Drum::Kick, 1.0)));
        let k2 = an.analyse(&kit_sound(|s| s.drum(Drum::Kick, 0.5)));
        let h = an.analyse(&kit_sound(|s| s.drum(Drum::Hat, 1.0)));
        assert!(cosine(&k1.emb, &k2.emb) > 0.95, "a kick is like itself at any level");
        assert!(cosine(&k1.emb, &k2.emb) > cosine(&k1.emb, &h.emb) + 0.2, "and unlike a hat");
    }

    fn library() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pmx_sorter_{}_{}", std::process::id(), rand_tag()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drums")).unwrap();
        let sounds: Vec<(&str, Vec<f32>)> = vec![
            ("drums/kick.wav", kit_sound(|s| s.drum(Drum::Kick, 1.0))),
            ("drums/kick soft.wav", kit_sound(|s| s.drum(Drum::Kick, 0.6))),
            ("drums/snare.wav", kit_sound(|s| s.drum(Drum::Snare, 1.0))),
            ("drums/hat.wav", kit_sound(|s| s.drum(Drum::Hat, 1.0))),
            ("drums/open.wav", kit_sound(|s| s.drum(Drum::OpenHat, 1.0))),
            ("drums/clap.wav", kit_sound(|s| s.drum(Drum::Clap, 1.0))),
            ("bass.wav", kit_sound(|s| s.play(Note::new(Tone::Bass, 33.0).len(0.7)))),
        ];
        for (p, a) in sounds {
            crate::apps::chop::write_wav(&dir.join(p), &a).unwrap();
        }
        dir
    }

    fn rand_tag() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 % 1_000_000
    }

    #[test]
    fn it_sorts_a_library_builds_a_kit_and_exports_it() {
        let dir = library();
        let index = dir.join("index.json");
        let mut s = Sorter::new(Sound::detached(), dir.clone(), Some(index.clone()));
        s.rescan();
        assert_eq!(s.s.total.load(Ordering::Relaxed), 7);
        s.analyse_all();
        s.refresh();
        let es = s.entries();
        assert_eq!(es.len(), 7);
        let kick = es.iter().position(|e| e.path.ends_with("kick.wav")).unwrap();
        assert_eq!(CLASSES[es[kick].a.class], "kick");
        // the nearest sound to the kick is the other kick
        s.select(kick);
        assert!(es[s.neighbours[0]].path.contains("kick soft"), "{}", es[s.neighbours[0]].path);
        // a kit around it: the kick pad gets a kick
        s.make_kit();
        assert!(s.kit[0].is_some_and(|i| es[i].a.class == 0));
        let out = s.export_kit().unwrap();
        let files: Vec<String> = std::fs::read_dir(&out).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(files.iter().any(|f| f.starts_with("01 kick")), "{files:?}");
        // remembered: a new Sorter doesn't need to listen again
        s.save_index();
        let mut s2 = Sorter::new(Sound::detached(), dir.clone(), Some(index));
        assert_eq!(s2.entries().len(), 7);
        s2.rescan();
        // only the exported kit's copies are new
        assert_eq!(s2.s.total.load(Ordering::Relaxed), files.len());
        // audition plays the file
        let mut p = s2.audio_processor().unwrap();
        s2.audition(0);
        let out = render(&mut p, 10);
        assert!(out.iter().any(|x| x.abs() > 0.01));
        let mut fb = FrameBuffer::new();
        for v in 0..3 {
            s2.view = v;
            s2.draw(&mut fb);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_map_walks_toward_where_you_point() {
        let mk = |x: f32, y: f32| Entry { path: format!("{x},{y}"), bytes: 0, modified: 0, a: Analysis { class: 0, probs: vec![1.0 / 14.0; 14], emb: vec![0.0; EMBED], secs: 0.1, hits: 1, is_loop: false } };
        let mut s = Sorter::new(Sound::detached(), PathBuf::from("/nonexistent"), None);
        *s.s.entries.lock().unwrap() = vec![mk(0.5, 0.5), mk(0.9, 0.5), mk(0.5, 0.9), mk(0.1, 0.5)];
        s.points = vec![(0.5, 0.5), (0.9, 0.5), (0.5, 0.9), (0.1, 0.5)];
        s.points_for = 4;
        s.cur = 0;
        s.walk(1.0, 0.0);
        assert_eq!(s.cur, 1);
        s.walk(-1.0, 0.0);
        assert_eq!(s.cur, 0);
        s.walk(0.0, 1.0);
        assert_eq!(s.cur, 2);
    }

    /// What it makes of a real library: `PORTAMAX_SORTER_DIR=path cargo
    /// test -- --ignored sorter_report --nocapture`. A report, not a pass
    /// mark -- real samples have no right answers on file.
    #[test]
    #[ignore]
    fn sorter_report() {
        let Some(dir) = std::env::var_os("PORTAMAX_SORTER_DIR").map(PathBuf::from) else { return };
        let mut files = Vec::new();
        scan(&dir, &mut files, 0);
        let mut an = Analyzer::new();
        for p in files {
            let Ok(a) = crate::apps::chop::read_wav(&p) else { continue };
            let r = an.analyse(&a);
            let mut top: Vec<(usize, f32)> = r.probs.iter().copied().enumerate().collect();
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            println!("{:60} {:5.2}s {:>3} hits {:4}  {:10} {:3.0}%   then {} {:.0}%", p.strip_prefix(&dir).unwrap_or(&p).display(), r.secs, r.hits, if r.is_loop { "LOOP" } else { "" }, CLASSES[top[0].0], top[0].1 * 100.0, CLASSES[top[1].0], top[1].1 * 100.0);
        }
    }

    #[test]
    fn loops_are_told_from_one_shots() {
        let mut an = Analyzer::new();
        // a two-bar beat from the kit: kick, hat, snare, hat ...
        let snd = Sound::detached();
        snd.set_reverb(0.0);
        let mut p = snd.processor(None, None);
        let mut beat = Vec::new();
        for step in 0..16 {
            snd.drum([Drum::Kick, Drum::Hat, Drum::Snare, Drum::Hat][step % 4], 1.0);
            beat.extend(render(&mut p, 12).chunks(2).map(|c| c[0]));
        }
        let a = an.analyse(&beat);
        assert!(a.is_loop, "{} hits in {:.2}s", a.hits, a.secs);
        // The kicks and snares are all found; hi-hats are mostly above
        // 8 kHz, beyond what 16 kHz analysis hears, so some go uncounted.
        assert!(a.hits >= 8, "{}", a.hits);
        // one kick with a long tail, and a long held organ chord: not loops
        let k = an.analyse(&kit_sound(|s| s.drum(Drum::Kick, 1.0)));
        assert!(!k.is_loop && k.hits <= 3, "kick: {} hits", k.hits);
        let pad = an.analyse(&kit_sound(|s| {
            for n in [60.0, 64.0, 67.0] {
                s.play(Note::new(Tone::Organ, n).len(0.75));
            }
        }));
        assert!(!pad.is_loop, "chord: {} hits", pad.hits);
        // and a kit never puts a loop on a pad
        let mk = |a: Analysis| Entry { path: String::new(), bytes: 0, modified: 0, a };
        let mut loop_kick = a.clone();
        loop_kick.class = 0;
        loop_kick.probs[0] = 1.0;
        let es = vec![mk(loop_kick)];
        assert_eq!(build_kit(&es, None, 0)[0], None);
    }

    /// 10,000 sounds in clusters, the way a real library lumps by kind.
    fn big_library(n: usize) -> Sorter {
        let mut r = 0x9E37_79B9u32;
        let mut rnd = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            (r >> 8) as f32 / 16_777_216.0
        };
        let mut es = Vec::new();
        let mut pts = Vec::new();
        let centres: Vec<(f32, f32)> = (0..N_CLASSES).map(|_| (0.1 + 0.8 * rnd(), 0.1 + 0.8 * rnd())).collect();
        for i in 0..n {
            let class = (rnd() * N_CLASSES as f32) as usize % N_CLASSES;
            let (cx, cy) = centres[class];
            let (gx, gy) = ((rnd() + rnd() + rnd() - 1.5) * 0.28, (rnd() + rnd() + rnd() - 1.5) * 0.28);
            pts.push(((cx + gx).clamp(0.0, 1.0), (cy + gy).clamp(0.0, 1.0)));
            let mut probs = vec![0.01; N_CLASSES];
            probs[class] = 0.8;
            es.push(Entry { path: format!("lib/{class}/sample {i}.wav"), bytes: 0, modified: 0, a: Analysis { class, probs, emb: vec![0.0; EMBED], secs: 0.05 + rnd() * rnd() * 4.0, hits: 1, is_loop: i % 9 == 0 } });
        }
        let mut s = Sorter::new(Sound::detached(), PathBuf::from("/nonexistent"), None);
        *s.s.entries.lock().unwrap() = es;
        s.points = pts;
        s.points_for = n;
        s.cur = n / 3;
        s.neighbours = (1..=16).map(|k| (n / 3 + k * 7) % n).collect();
        s
    }

    fn dump(fb: &FrameBuffer, path: &str) {
        let mut out = format!("P6\n{} {}\n255\n", crate::display::WIDTH, crate::display::HEIGHT).into_bytes();
        for c in fb.buffer() {
            out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, *c as u8]);
        }
        std::fs::write(path, out).unwrap();
    }

    /// With 10,000 sounds the map still draws in a frame's time, at every
    /// zoom. Set SORTER_PPM_DIR to keep the frames for a look.
    #[test]
    fn a_ten_thousand_sound_map_zooms_and_draws_fast() {
        let mut s = big_library(10_000);
        let mut fb = FrameBuffer::new();
        for (name, z) in [("zoom1", 1.0f32), ("zoom6", 6.0), ("zoom30", 30.0)] {
            s.zoom_target = z;
            for _ in 0..40 {
                s.ease_view();
            }
            assert!((s.zoom - z).abs() < 0.05, "zoom eases to its target");
            s.draw(&mut fb);
            let t = Instant::now();
            for _ in 0..5 {
                s.draw(&mut fb);
            }
            let ms = t.elapsed().as_secs_f32() * 200.0;
            eprintln!("{name}: {ms:.1} ms a frame");
            assert!(ms < 30.0, "{name} took {ms} ms");
            if let Ok(dir) = std::env::var("SORTER_PPM_DIR") {
                dump(&fb, &format!("{dir}/{name}.ppm"));
            }
        }
    }

    #[test]
    fn zoom_is_clamped_and_the_view_stays_on_the_map() {
        let mut s = big_library(500);
        s.zoom_target = 9999.0;
        s.zoom_target = s.zoom_target.clamp(1.0, MAX_ZOOM);
        for _ in 0..60 {
            s.ease_view();
        }
        assert!((s.zoom - MAX_ZOOM).abs() < 0.5);
        let half = 0.5 / s.zoom;
        assert!(s.pan.0 >= half - 0.01 && s.pan.0 <= 1.0 - half + 0.01 && s.pan.1 >= half - 0.01 && s.pan.1 <= 1.0 - half + 0.01);
        // the sound you're on is on screen
        let (x, y) = s.to_map(s.points[s.cur]);
        assert!(x >= 0.0 && x <= MAP_W as f32 && y >= 0.0 && y <= MAP_H as f32, "({x}, {y})");
        // and the knob zooms it in a step at a time
        let mut s = big_library(100);
        s.view = 0;
        s.tick(&Input { knob1: 1, ..Default::default() });
        assert!(s.zoom_target > 1.4 && s.zoom_target < 1.6);
        s.tick(&Input { knob1: -3, ..Default::default() });
        assert_eq!(s.zoom_target, 1.0);
    }

    #[test]
    fn the_library_scan_takes_more_than_the_old_5000() {
        assert!(MAX_FILES >= 20_000);
        assert_eq!(thousands(10402), "10,402");
        assert_eq!(thousands(12), "12");
    }
}
