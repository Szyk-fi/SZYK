//! 5 real spectral/signal analyzers over a self-generated demo signal.
//!
//! Honest limitation: there's no app-linking yet (see main.rs's module
//! doc for the "4 linked apps" direction), and no microphone input in
//! this build, so there's nothing external to analyze. This app
//! generates its own signal (sine/noise/sweep, picked from the list) and
//! sends it to *both* the speakers and the analyzers -- so what you see
//! is what you hear, and once app-linking exists this is the natural
//! place to swap the demo oscillator for "whatever the linked app is
//! playing."
//!
//! Modes (list item "Mode"):
//!   0. Spectrum   -- FFT magnitude bars (linear frequency bins, dB scale)
//!   1. Oscilloscope -- raw waveform trace
//!   2. Spectrogram -- scrolling time/frequency heatmap (reuses the same FFT)
//!   3. Level meter -- peak (with hold) + RMS, in dB
//!   4. Pitch detect -- autocorrelation fundamental frequency -> note name
//!
//! Control surface matches Plaits' new pattern: knob1 navigates the
//! (short) settings list, knob2 edits the selected item.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::paramlist::ParamList;
use crate::util::{note_name, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_8X16};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const FFT_SIZE: usize = 1024;
const NUM_BARS: usize = 80;
const WAVE_LEN: usize = 512;
const SPECTROGRAM_COLUMNS: usize = 160;
const MIN_DB: f32 = -60.0;
const MAX_DB: f32 = 0.0;
/// Sentinel for "no pitch detected" in the shared AtomicI32 (real note
/// numbers comfortably fit in a much smaller range).
const NO_PITCH: i32 = i32::MIN;

const MODE_NAMES: [&str; 5] = ["Spectrum", "Oscilloscope", "Spectrogram", "Level Meter", "Pitch Detect"];
const DEMO_NAMES: [&str; 3] = ["Sine", "Noise", "Sweep"];

struct Shared {
    mode: AtomicU32,
    demo_kind: AtomicU32,
    frequency: AtomicF32,
    spectrum: Mutex<Vec<f32>>,      // dB per bar, NUM_BARS long
    waveform: Mutex<Vec<f32>>,      // raw samples, WAVE_LEN long
    spectrogram: Mutex<VecDeque<Vec<u8>>>, // columns of NUM_BARS intensities (0..255)
    peak_db: AtomicF32,
    rms_db: AtomicF32,
    detected_note: AtomicI32,
}

impl Shared {
    fn new() -> Self {
        Self {
            mode: AtomicU32::new(0),
            demo_kind: AtomicU32::new(0),
            frequency: AtomicF32::new(220.0),
            spectrum: Mutex::new(vec![MIN_DB; NUM_BARS]),
            waveform: Mutex::new(vec![0.0; WAVE_LEN]),
            spectrogram: Mutex::new(VecDeque::new()),
            peak_db: AtomicF32::new(MIN_DB),
            rms_db: AtomicF32::new(MIN_DB),
            detected_note: AtomicI32::new(NO_PITCH),
        }
    }
}

enum Selection {
    Mode,
    DemoKind,
    Frequency,
}

fn selection_for(index: usize) -> Selection {
    match index {
        0 => Selection::Mode,
        1 => Selection::DemoKind,
        _ => Selection::Frequency,
    }
}
const NUM_ROWS: usize = 3;

pub struct AnalyzerApp {
    shared: Arc<Shared>,
    list: ParamList,
}

// --- Analyzer's own palette: clinical phosphor cyan on near-black,
// not a device-wide theme -- the look of a real CRT lab scope, cold
// and precise, matching this app's own "measure the truth of a
// signal" character. A real clip warning still flashes red
// (`draw_db_bar`) -- that's a functional alert, not this palette. ---

const ANALYZER_BG: Rgb565 = Rgb565::new(0, 2, 1);
const ANALYZER_TITLE: Rgb565 = Rgb565::new(26, 60, 31);
const ANALYZER_ACCENT: Rgb565 = Rgb565::new(0, 56, 31);
const ANALYZER_DIM: Rgb565 = Rgb565::new(9, 22, 12);

impl AnalyzerApp {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared::new()),
            list: ParamList::new(),
        }
    }

    /// Real `(name, value)` rows for this flat 3-row list -- exposed
    /// for an alternate renderer (a live Slint screen) instead of
    /// drawn. No groups here (unlike every other app), so there's no
    /// `is_group`/windowing to do -- always all 3 rows.
    pub(crate) fn display_rows(&self) -> Vec<(String, String)> {
        let mode = self.shared.mode.load(Ordering::Relaxed) as usize % MODE_NAMES.len();
        let demo = self.shared.demo_kind.load(Ordering::Relaxed) as usize % DEMO_NAMES.len();
        vec![
            ("Mode".to_string(), MODE_NAMES[mode].to_string()),
            ("Demo Signal".to_string(), DEMO_NAMES[demo].to_string()),
            ("Frequency".to_string(), format!("{:.0} Hz", self.shared.frequency.get())),
        ]
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    /// Which of `MODE_NAMES` the display should currently render.
    pub(crate) fn mode_kind(&self) -> (u32, &'static str) {
        let idx = self.shared.mode.load(Ordering::Relaxed) % MODE_NAMES.len() as u32;
        (idx, MODE_NAMES[idx as usize])
    }

    /// Real FFT spectrum bars, normalized 0..1 -- same source the
    /// real Spectrum/Spectrogram panels read.
    pub(crate) fn spectrum_levels(&self) -> Vec<f32> {
        self.shared.spectrum.lock().unwrap().iter().map(|db| ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0)).collect()
    }

    /// Real time-domain waveform, -1..1.
    pub(crate) fn waveform_samples(&self) -> Vec<f32> {
        self.shared.waveform.lock().unwrap().iter().map(|v| v.clamp(-1.0, 1.0)).collect()
    }

    /// Real peak/RMS level, normalized 0..1 -- `(peak, rms)`.
    pub(crate) fn level_meters(&self) -> (f32, f32) {
        let norm = |db: f32| ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
        (norm(self.shared.peak_db.get()), norm(self.shared.rms_db.get()))
    }

    /// The autocorrelation-detected note name, or "--".
    pub(crate) fn detected_note_name(&self) -> String {
        let note = self.shared.detected_note.load(Ordering::Relaxed);
        if note == NO_PITCH { "--".to_string() } else { crate::util::note_name(note) }
    }
}

impl App for AnalyzerApp {
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows().into_iter().map(|(n, v)| (n, v, false)).collect()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let (analyzer_kind, analyzer_name) = self.mode_kind();
        let mut spectrum = Vec::new();
        let mut waveform = crate::app::CurveSegments::default();
        let mut peak_level = 0.0;
        let mut rms_level = 0.0;
        let mut pitch_name = String::new();
        match analyzer_kind {
            0 => spectrum = self.spectrum_levels(),
            1 => {
                // Real connected-line-segment geometry (see
                // `polyline_segments`), not a bar chart -- the fixed
                // canvas size here must match what the live screen
                // actually draws this panel at.
                const SCOPE_POINTS: usize = 60;
                const PANEL_W: f32 = 260.0;
                const PANEL_H: f32 = 190.0;
                let raw = self.waveform_samples();
                if !raw.is_empty() {
                    let step = (raw.len() as f32 / SCOPE_POINTS as f32).max(1.0);
                    let resampled: Vec<f32> = (0..SCOPE_POINTS).map(|i| raw[((i as f32 * step) as usize).min(raw.len() - 1)]).collect();
                    let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&resampled, PANEL_W, PANEL_H, true);
                    waveform = crate::app::CurveSegments { mid_x, mid_y, length, angle_deg };
                }
            }
            // 2 = Spectrogram -- not rendered live here (see
            // slint_analyzer_live.rs's own copy of this same
            // scope-cut decision); falls through with everything
            // empty, which the Slint side shows as a plain label.
            3 => (peak_level, rms_level) = self.level_meters(),
            4 => pitch_name = self.detected_note_name(),
            _ => {}
        }
        crate::app::SlintExtra::Analyzer(crate::app::AnalyzerExtra {
            analyzer_kind,
            analyzer_name: analyzer_name.to_string(),
            spectrum,
            waveform,
            peak_level,
            rms_level,
            pitch_name,
        })
    }

    fn tick(&mut self, input: &Input) {
        // No groups to expand/collapse here -- unlike every other
        // app's menu, this one is a flat 3-row list, so knob1_press
        // (which elsewhere toggles a group) simply does nothing,
        // rather than the surprising "jump back to row 0" this used
        // to do.
        self.list.navigate_input(input, NUM_ROWS, 1);

        if input.knob2 != 0 {
            match selection_for(self.list.selected) {
                Selection::Mode => {
                    let cur = self.shared.mode.load(Ordering::Relaxed) as i32;
                    let next = (cur + input.knob2).rem_euclid(MODE_NAMES.len() as i32);
                    self.shared.mode.store(next as u32, Ordering::Relaxed);
                }
                Selection::DemoKind => {
                    let cur = self.shared.demo_kind.load(Ordering::Relaxed) as i32;
                    let next = (cur + input.knob2).rem_euclid(DEMO_NAMES.len() as i32);
                    self.shared.demo_kind.store(next as u32, Ordering::Relaxed);
                }
                Selection::Frequency => {
                    let next = (self.shared.frequency.get() * 1.05f32.powi(input.knob2)).clamp(20.0, 8000.0);
                    self.shared.frequency.set(next);
                }
            }
        }
        if input.knob2_press {
            match selection_for(self.list.selected) {
                Selection::Frequency => self.shared.frequency.set(220.0),
                Selection::Mode => self.shared.mode.store(0, Ordering::Relaxed),
                Selection::DemoKind => self.shared.demo_kind.store(0, Ordering::Relaxed),
            }
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        Some(Box::new(AnalyzerProcessor {
            shared: Arc::clone(&self.shared),
            fft,
            history: VecDeque::with_capacity(FFT_SIZE),
            phase: 0.0,
            sweep_phase: 0.0,
            rng: 0xC0FFEE,
            peak_hold: MIN_DB,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(ANALYZER_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, ANALYZER_TITLE);
        Text::new("Analyzer", Point::new(16, 26), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_8X16, ANALYZER_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_8X16, ANALYZER_DIM);

        let mode = self.shared.mode.load(Ordering::Relaxed) as usize % MODE_NAMES.len();
        let demo = self.shared.demo_kind.load(Ordering::Relaxed) as usize % DEMO_NAMES.len();
        let rows = vec![
            ("Mode".to_string(), MODE_NAMES[mode].to_string()),
            ("Demo Signal".to_string(), DEMO_NAMES[demo].to_string()),
            ("Frequency".to_string(), format!("{:.0} Hz", self.shared.frequency.get())),
        ];
        self.list.draw_themed(fb, 16, 56, 26, 3, &rows, ANALYZER_BG, ANALYZER_DIM, ANALYZER_ACCENT);

        let plot_x = 16;
        let plot_y = 150;
        let plot_w = 608;
        let plot_h = 190;

        match mode {
            0 => self.draw_spectrum(fb, plot_x, plot_y, plot_w, plot_h),
            1 => self.draw_waveform(fb, plot_x, plot_y, plot_w, plot_h),
            2 => self.draw_spectrogram(fb, plot_x, plot_y, plot_w, plot_h),
            3 => self.draw_level(fb, plot_x, plot_y, plot_w, accent, dim),
            _ => self.draw_pitch(fb, plot_x, plot_y, accent, dim),
        }
    }
}

impl AnalyzerApp {
    fn draw_spectrum(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32) {
        let spectrum = self.shared.spectrum.lock().unwrap();
        let bar_w = (w / NUM_BARS as i32).max(1);
        for (i, db) in spectrum.iter().enumerate() {
            let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
            let bar_h = (t * h as f32) as u32;
            if bar_h == 0 {
                continue;
            }
            let bx = x + i as i32 * bar_w;
            let by = y + h - bar_h as i32;
            Rectangle::new(Point::new(bx, by), Size::new((bar_w - 1).max(1) as u32, bar_h))
                .into_styled(PrimitiveStyle::with_fill(ANALYZER_ACCENT))
                .draw(fb)
                .ok();
        }
    }

    fn draw_waveform(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32) {
        let wave = self.shared.waveform.lock().unwrap();
        let mid = y + h / 2;
        let mut prev: Option<Point> = None;
        for (i, sample) in wave.iter().enumerate() {
            let px = x + (i as i32 * w) / wave.len() as i32;
            let py = mid - (sample.clamp(-1.0, 1.0) * (h as f32 / 2.0)) as i32;
            let point = Point::new(px, py);
            if let Some(p0) = prev {
                draw_line(fb, p0, point, ANALYZER_ACCENT);
            }
            prev = Some(point);
        }
    }

    fn draw_spectrogram(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32) {
        let columns = self.shared.spectrogram.lock().unwrap();
        if columns.is_empty() {
            return;
        }
        let col_w = (w / SPECTROGRAM_COLUMNS as i32).max(1);
        let row_h = (h / NUM_BARS as i32).max(1);
        for (ci, col) in columns.iter().enumerate() {
            let cx = x + ci as i32 * col_w;
            for (ri, &intensity) in col.iter().enumerate() {
                if intensity == 0 {
                    continue;
                }
                let cy = y + h - (ri as i32 + 1) * row_h;
                // Cyan heat instead of green -- both G and B climb
                // with intensity so a hot cell reads white-cyan, not
                // acid green, matching this app's own scope palette.
                let g = (intensity as u32 * 63 / 255) as u8;
                let b = (intensity as u32 * 31 / 255) as u8;
                Rectangle::new(Point::new(cx, cy), Size::new(col_w as u32, row_h as u32))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(1, g, b)))
                    .draw(fb)
                    .ok();
            }
        }
    }

    fn draw_level(
        &self,
        fb: &mut FrameBuffer,
        x: i32,
        y: i32,
        w: i32,
        accent: MonoTextStyle<'_, Rgb565>,
        dim: MonoTextStyle<'_, Rgb565>,
    ) {
        let peak_db = self.shared.peak_db.get();
        let rms_db = self.shared.rms_db.get();

        let bar_h = 40;
        Text::new(&format!("Peak: {peak_db:.1} dB"), Point::new(x, y - 10), accent)
            .draw(fb)
            .ok();
        draw_db_bar(fb, x, y + 10, w, bar_h, peak_db);
        Text::new(&format!("RMS: {rms_db:.1} dB"), Point::new(x, y + 70), dim)
            .draw(fb)
            .ok();
        draw_db_bar(fb, x, y + 90, w, bar_h, rms_db);
    }

    fn draw_pitch(&self, fb: &mut FrameBuffer, x: i32, y: i32, accent: MonoTextStyle<'_, Rgb565>, dim: MonoTextStyle<'_, Rgb565>) {
        let note = self.shared.detected_note.load(Ordering::Relaxed);
        if note == NO_PITCH {
            Text::new("No pitch detected", Point::new(x, y + 40), dim).draw(fb).ok();
        } else {
            let big = MonoTextStyle::new(&SPLEEN_16X32, ANALYZER_ACCENT);
            Text::new(&note_name(note), Point::new(x, y + 40), big).draw(fb).ok();
            let _ = accent;
        }
    }
}

fn draw_db_bar(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, db: f32) {
    Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32))
        .into_styled(PrimitiveStyle::with_stroke(ANALYZER_DIM, 1))
        .draw(fb)
        .ok();
    let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
    let fill_w = (t * w as f32) as u32;
    if fill_w > 0 {
        // A real clip warning still flashes red regardless of this
        // app's own cyan palette -- that's a functional alert, not
        // personality styling.
        let color = if db > -3.0 { Rgb565::new(31, 10, 10) } else { ANALYZER_ACCENT };
        Rectangle::new(Point::new(x, y), Size::new(fill_w, h as u32))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(fb)
            .ok();
    }
}

/// Simple interpolated line for the oscilloscope trace.
fn draw_line(fb: &mut FrameBuffer, p0: Point, p1: Point, color: Rgb565) {
    let (x0, y0, x1, y1) = (p0.x, p0.y, p1.x, p1.y);
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let steps = dx.max(dy).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x0 + ((x1 - x0) as f32 * t) as i32;
        let y = y0 + ((y1 - y0) as f32 * t) as i32;
        fb.draw_iter(core::iter::once(embedded_graphics::Pixel(Point::new(x, y), color)))
            .ok();
    }
}

struct AnalyzerProcessor {
    shared: Arc<Shared>,
    fft: Arc<dyn Fft<f32>>,
    history: VecDeque<f32>,
    phase: f32,
    sweep_phase: f32,
    rng: u32,
    peak_hold: f32,
}

impl AnalyzerProcessor {
    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn analyze(&mut self, sample_rate: f32) {
        if self.history.len() < FFT_SIZE {
            return;
        }
        // Windowed (Hann) copy for the FFT; the raw history stays as-is
        // for the oscilloscope/level/pitch analyses below.
        let mut buf: Vec<Complex<f32>> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let w = 0.5 - 0.5 * (TAU * i as f32 / (FFT_SIZE - 1) as f32).cos();
                Complex::new(*s * w, 0.0)
            })
            .collect();
        self.fft.process(&mut buf);

        let bins_per_bar = (FFT_SIZE / 2) / NUM_BARS;
        let mut bars = vec![MIN_DB; NUM_BARS];
        for (bar, bin_group) in bars.iter_mut().zip(buf[..FFT_SIZE / 2].chunks(bins_per_bar.max(1))) {
            let mag = bin_group.iter().map(|c| c.norm()).fold(0.0f32, f32::max);
            let db = 20.0 * (mag / FFT_SIZE as f32 + 1e-9).log10();
            *bar = db.clamp(MIN_DB, MAX_DB);
        }
        *self.shared.spectrum.lock().unwrap() = bars.clone();

        let column: Vec<u8> = bars
            .iter()
            .map(|db| (((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0) * 255.0) as u8)
            .collect();
        let mut spectrogram = self.shared.spectrogram.lock().unwrap();
        spectrogram.push_back(column);
        while spectrogram.len() > SPECTROGRAM_COLUMNS {
            spectrogram.pop_front();
        }
        drop(spectrogram);

        // Autocorrelation pitch detection over the same window.
        let samples: Vec<f32> = self.history.iter().copied().collect();
        let energy: f32 = samples.iter().map(|s| s * s).sum();
        let min_lag = (sample_rate / 2000.0).max(2.0) as usize;
        let max_lag = ((sample_rate / 40.0) as usize).min(samples.len() - 1);
        let mut best_lag = 0;
        let mut best_corr = 0.0f32;
        if max_lag > min_lag {
            for lag in min_lag..max_lag {
                let mut corr = 0.0;
                for i in 0..(samples.len() - lag) {
                    corr += samples[i] * samples[i + lag];
                }
                if corr > best_corr {
                    best_corr = corr;
                    best_lag = lag;
                }
            }
        }
        if best_lag > 0 && energy > 1e-6 && best_corr / energy > 0.35 {
            let freq = sample_rate / best_lag as f32;
            let note = (12.0 * (freq / 440.0).log2() + 69.0).round() as i32;
            self.shared.detected_note.store(note, Ordering::Relaxed);
        } else {
            self.shared.detected_note.store(NO_PITCH, Ordering::Relaxed);
        }
    }
}

impl AudioProcessor for AnalyzerProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let demo_kind = self.shared.demo_kind.load(Ordering::Relaxed);
        let base_freq = self.shared.frequency.get();

        let frames = buffer.len() / channels;
        let mut mono = vec![0.0f32; frames];
        for sample in mono.iter_mut() {
            let value = match demo_kind {
                1 => self.next_rand() * 0.3,
                2 => {
                    // Slow sweep from base_freq up to ~4x and back.
                    self.sweep_phase = (self.sweep_phase + 0.00002) % 2.0;
                    let sweep_t = if self.sweep_phase < 1.0 { self.sweep_phase } else { 2.0 - self.sweep_phase };
                    let freq = base_freq * (1.0 + sweep_t * 3.0);
                    self.phase = (self.phase + freq / sample_rate).fract();
                    (self.phase * TAU).sin() * 0.4
                }
                _ => {
                    self.phase = (self.phase + base_freq / sample_rate).fract();
                    (self.phase * TAU).sin() * 0.4
                }
            };
            *sample = value;
        }

        for (frame, sample) in buffer.chunks_mut(channels).zip(mono.iter()) {
            for out in frame.iter_mut() {
                *out = *sample;
            }
        }

        // Feed the analysis history (used by every mode).
        for s in &mono {
            self.history.push_back(*s);
            if self.history.len() > FFT_SIZE {
                self.history.pop_front();
            }
        }

        // Waveform (oscilloscope) -- just the most recent WAVE_LEN samples.
        {
            let mut wave = self.shared.waveform.lock().unwrap();
            wave.clear();
            wave.extend(self.history.iter().rev().take(WAVE_LEN).rev());
            while wave.len() < WAVE_LEN {
                wave.insert(0, 0.0);
            }
        }

        // Level meter: peak (with slow hold decay) and RMS over this block.
        let block_peak = mono.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let block_peak_db = 20.0 * (block_peak + 1e-9).log10();
        self.peak_hold = (self.peak_hold - 0.5).max(block_peak_db).max(MIN_DB);
        self.shared.peak_db.set(self.peak_hold.clamp(MIN_DB, MAX_DB));
        let rms = (mono.iter().map(|s| s * s).sum::<f32>() / frames.max(1) as f32).sqrt();
        let rms_db = (20.0 * (rms + 1e-9).log10()).clamp(MIN_DB, MAX_DB);
        self.shared.rms_db.set(rms_db);

        self.analyze(sample_rate);
    }
}
