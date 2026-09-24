//! A real slot in the app registry, not a separate standalone tool --
//! this replaces the earlier `examples/te_visualizer.rs` desktop
//! prototype (removed) with the same idea folded into the simulator
//! itself: knob1 navigates a flat menu (Mode, Demo Signal, Frequency,
//! Scene), knob2 edits the selected row, exactly like every other app
//! here. No mouse-only controls, no separate window.
//!
//! Wraps `AnalyzerApp` verbatim for the real signal generation/
//! analysis (self-generated sine/noise/sweep, real FFT spectrum, real
//! oscilloscope trace, real peak/RMS meters, real autocorrelation
//! pitch detection) and adds one more row on top, "Scene", which
//! swaps the plot area for one of two audio-reactive pixel-art
//! "mascots" instead of a plain scope/spectrum readout -- the same
//! idea as the little animated icons Teenage Engineering's own
//! hardware shows on its screen (OP-1 synth-engine icons, TX-6 VU
//! pixel art), not a literal reproduction of either product:
//!   - **Boxer**: bobs on bass energy, jabs both gloves outward on a
//!     beat hit.
//!   - **Car**: drives left-to-right on a loop, hops on a beat,
//!     headlight/rear glow trail brighten with treble energy.
//!   - **Cat**: a maneki-neko DJ -- head/ears nod and its raised paw
//!     swings side to side on every beat (a real 4/4 will read as a
//!     steady metronome swing since it flips side on each beat
//!     onset, not a fixed tempo), boombox shows a tiny live spectrum
//!     readout.
//! All three read real numbers off `AnalyzerApp` (bass/treble = averaged
//! low/high spectrum bins, "beat" = a real peak-level threshold
//! crossing that spikes then decays) -- see `tick`'s bookkeeping.
//!
//! `AnalyzerApp`'s own selection state is private (`list`/`shared`
//! aren't `pub`), so editing one of its first 3 rows works by
//! "walking" its internal cursor onto the target row with repeated
//! `knob1: 1` ticks before forwarding the real `knob2` edit -- the
//! same trick `examples/te_visualizer.rs` used against the same
//! constraint. Its own audio processing (FFT/level calc) runs
//! independent of menu navigation, so Scene reads accurate
//! spectrum/level data no matter which row is selected.
//!
//! Next step once this is playing well: swap `AnalyzerApp`'s self-
//! generated demo signal for a real tap into this sim's `AudioBus`,
//! so Boxer/Car react to whichever app (or the full master mix) is
//! actually playing, instead of the demo oscillator's incidental
//! level wobble.
//!
//! **Monitor (default Off)**: `MixBus` runs every registered app's
//! processor every block for the life of the program, regardless of
//! which screen is active (see audio.rs's own doc comment) -- that's
//! right for an instrument that stays silent until played, but
//! `AnalyzerApp` is a self-contained tone generator that *always*
//! emits its demo signal by design (fine for the old standalone
//! prototype, where it was the only thing running). Wrapped into the
//! real registry unguarded, that meant a sine tone humming the
//! instant the whole simulator booted, before you'd even opened this
//! app. `VisualizerProcessor` below gates the actual audible output
//! on `monitor` (an `Arc<AtomicBool>`, default `false`) while leaving
//! the underlying analysis running regardless -- Boxer/Car/scope keep
//! reacting to the demo signal even while it's muted, since Shared's
//! spectrum/level/waveform state updates inside `AnalyzerApp`'s own
//! processor regardless of what we do to its output afterward.

use super::analyzer::AnalyzerApp;
use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const SCENE_NAMES: [&str; 4] = ["Off", "Boxer", "Car", "Cat"];
const NUM_ROWS: usize = 5;
/// A real peak-level crossing above this counts as a "beat" -- see
/// `tick`'s beat-pulse bookkeeping.
const BEAT_THRESHOLD: f32 = 0.5;

// --- Visualizer's own palette: warm cream and bold orange, not a
// device-wide theme -- the same Teenage-Engineering-inspired look
// this app was designed around from the start (see the module doc
// comment), now actually applied to the real device screen instead
// of only its Slint live-preview panel. Shares its exact hues with
// Bloom's own cream/orange redesign -- both apps are explicitly
// TE-inspired, so a shared family here is deliberate, not an
// oversight. ---

const VISUALIZER_BG: Rgb565 = Rgb565::new(18, 36, 17);
const VISUALIZER_TITLE: Rgb565 = Rgb565::new(4, 6, 2);
const VISUALIZER_ACCENT: Rgb565 = Rgb565::new(11, 8, 1);
const VISUALIZER_DIM: Rgb565 = Rgb565::new(5, 9, 4);
const VISUALIZER_CHIP_BG: Rgb565 = VISUALIZER_ACCENT;
/// The Boxer/Car scenes' own ground line, gloves' dim outline, etc.
const VISUALIZER_SCENE_DIM: Rgb565 = Rgb565::new(17, 30, 11);
/// The Car scene's headlight -- a brighter flash than the base accent.
const VISUALIZER_HEADLIGHT: Rgb565 = Rgb565::new(31, 30, 7);

pub struct VisualizerApp {
    analyzer: AnalyzerApp,
    list: ParamList,
    scene: usize,
    /// Gates the actual audible output -- see the module doc
    /// comment's "Monitor" section. Shared with `VisualizerProcessor`
    /// on the audio thread.
    monitor: Arc<AtomicBool>,
    /// Spikes to 1.0 on a beat, decays every frame -- drives both
    /// scenes' punchy hit reactions.
    beat_pulse: f32,
    /// 0..1 fraction across the plot width, crawls forward every
    /// frame and wraps -- the Car scene's looping drive position.
    car_x: f32,
    /// Which side the Cat scene's raised paw is currently swung to --
    /// flips on every beat onset (see `tick`), so it swings side to
    /// side in time with a real 4/4 beat like a metronome.
    cat_paw_left: bool,
    last_tick: Instant,
}

impl VisualizerApp {
    pub fn new() -> Self {
        Self {
            analyzer: AnalyzerApp::new(),
            list: ParamList::new(),
            scene: 0,
            monitor: Arc::new(AtomicBool::new(false)),
            beat_pulse: 0.0,
            car_x: 0.0,
            cat_paw_left: true,
            last_tick: Instant::now(),
        }
    }

    fn display_rows(&self) -> Vec<(String, String)> {
        let mut rows = self.analyzer.display_rows();
        rows.push(("Scene".to_string(), SCENE_NAMES[self.scene % SCENE_NAMES.len()].to_string()));
        rows.push(("Monitor".to_string(), if self.monitor.load(Ordering::Relaxed) { "On".to_string() } else { "Off".to_string() }));
        rows
    }
}

/// Wraps `AnalyzerApp`'s own processor to gate its audible output on
/// `monitor` -- see the module doc comment's "Monitor" section. Its
/// internal analysis (spectrum/level/waveform) still runs every block
/// regardless, since that happens inside `inner.process` before the
/// gate is applied.
struct VisualizerProcessor {
    inner: Box<dyn AudioProcessor>,
    monitor: Arc<AtomicBool>,
}

impl AudioProcessor for VisualizerProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        self.inner.process(buffer, channels, sample_rate);
        if !self.monitor.load(Ordering::Relaxed) {
            for s in buffer.iter_mut() {
                *s = 0.0;
            }
        }
    }
}

impl App for VisualizerApp {
    fn tick(&mut self, input: &Input) {
        self.list.navigate_input(input, NUM_ROWS, 1);
        let row = self.list.selected;

        if input.knob2 != 0 {
            if row < 3 {
                let mut guard = 0;
                while self.analyzer.selected_row() != row && guard < 8 {
                    self.analyzer.tick(&Input { knob1: 1, ..Default::default() });
                    guard += 1;
                }
                self.analyzer.tick(&Input { knob2: input.knob2, ..Default::default() });
            } else if row == 3 {
                let next = (self.scene as i32 + input.knob2).rem_euclid(SCENE_NAMES.len() as i32);
                self.scene = next as usize;
            } else {
                self.monitor.store(input.knob2 > 0, Ordering::Relaxed);
            }
        }
        if input.knob2_press {
            if row == 3 {
                self.scene = 0;
            } else if row == 4 {
                self.monitor.store(false, Ordering::Relaxed);
            }
        }

        // Scene animation/beat bookkeeping -- every frame regardless
        // of which row is selected, so Boxer/Car keep moving even
        // while you're editing Mode/Demo/Frequency.
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;
        self.car_x = (self.car_x + dt * 0.15) % 1.0;
        let (peak, _) = self.analyzer.level_meters();
        if peak > BEAT_THRESHOLD && self.beat_pulse < 0.2 {
            self.beat_pulse = 1.0;
            self.cat_paw_left = !self.cat_paw_left;
        } else {
            // Frame-rate-independent exponential decay: ~0.85 per
            // 33ms frame, scaled to whatever `dt` actually was.
            self.beat_pulse *= 0.85f32.powf(dt / 0.033);
        }
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows().into_iter().map(|(n, v)| (n, v, false)).collect()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let inner = self.analyzer.audio_processor()?;
        Some(Box::new(VisualizerProcessor { inner, monitor: Arc::clone(&self.monitor) }))
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let (mode_kind, mode_name) = self.analyzer.mode_kind();
        let mut spectrum = Vec::new();
        let mut waveform = crate::app::CurveSegments::default();
        let mut peak_level = 0.0;
        let mut rms_level = 0.0;
        let mut pitch_name = String::new();
        if self.scene == 0 {
            match mode_kind {
                0 => spectrum = self.analyzer.spectrum_levels(),
                1 => {
                    const SCOPE_POINTS: usize = 48;
                    const PANEL_W: f32 = 260.0;
                    const PANEL_H: f32 = 90.0;
                    let raw = self.analyzer.waveform_samples();
                    if !raw.is_empty() {
                        let step = (raw.len() as f32 / SCOPE_POINTS as f32).max(1.0);
                        let resampled: Vec<f32> = (0..SCOPE_POINTS).map(|i| raw[((i as f32 * step) as usize).min(raw.len() - 1)]).collect();
                        let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&resampled, PANEL_W, PANEL_H, true);
                        waveform = crate::app::CurveSegments { mid_x, mid_y, length, angle_deg };
                    }
                }
                3 => (peak_level, rms_level) = self.analyzer.level_meters(),
                4 => pitch_name = self.analyzer.detected_note_name(),
                _ => {}
            }
        } else if self.scene == 3 {
            // The Cat scene's boombox shows a tiny live spectrum
            // readout in the Slint panel too, same as the real device
            // screen's `draw_cat` -- reuses the same field the Off/
            // Spectrum mode above populates.
            spectrum = self.analyzer.spectrum_levels();
        }
        let (bass_level, treble_level) = bass_treble(&self.analyzer.spectrum_levels());
        crate::app::SlintExtra::Visualizer(crate::app::VisualizerExtra {
            mode_kind,
            mode_name: mode_name.to_string(),
            spectrum,
            waveform,
            peak_level,
            rms_level,
            pitch_name,
            scene_kind: self.scene as u32,
            scene_name: SCENE_NAMES[self.scene % SCENE_NAMES.len()].to_string(),
            bass_level,
            treble_level,
            beat_pulse: self.beat_pulse,
            car_x: self.car_x,
            cat_paw_left: self.cat_paw_left,
            monitor_on: self.monitor.load(Ordering::Relaxed),
        })
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(VISUALIZER_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, VISUALIZER_TITLE);
        Text::new("Visualizer", Point::new(16, 26), title).draw(fb).ok();

        let rows = self.display_rows();
        let row_h = 26;
        self.list.draw_themed(fb, 16, 56, row_h, rows.len(), &rows, VISUALIZER_BG, VISUALIZER_DIM, VISUALIZER_CHIP_BG);

        let x = 16;
        // Below the list, whatever its real row count is -- a
        // hardcoded row count here previously fell out of sync with
        // `rows.len()` (missing the newly added Monitor row) and let
        // the plot area creep up to overlap the list's last row.
        let y = (56 + rows.len() as i32 * row_h + 10).max(150);
        let w = 608;
        let h = 340 - y;

        if self.scene == 0 {
            let (mode, _) = self.analyzer.mode_kind();
            let dim = MonoTextStyle::new(&SPLEEN_6X12, VISUALIZER_DIM);
            match mode {
                0 => draw_spectrum(fb, &self.analyzer.spectrum_levels(), x, y, w, h),
                1 => draw_waveform(fb, &self.analyzer.waveform_samples(), x, y, w, h),
                3 => {
                    let (peak, rms) = self.analyzer.level_meters();
                    draw_levels(fb, peak, rms, x, y, w);
                }
                4 => {
                    let big = MonoTextStyle::new(&SPLEEN_16X32, VISUALIZER_ACCENT);
                    Text::new(&self.analyzer.detected_note_name(), Point::new(x, y + 40), big).draw(fb).ok();
                }
                _ => {
                    Text::new("(spectrogram not rendered here -- see Spectrum)", Point::new(x, y + 20), dim).draw(fb).ok();
                }
            }
        } else if self.scene == 1 {
            draw_boxer(fb, x, y, w, h, &self.analyzer.spectrum_levels(), self.beat_pulse);
        } else if self.scene == 2 {
            draw_car(fb, x, y, w, h, &self.analyzer.spectrum_levels(), self.beat_pulse, self.car_x);
        } else {
            draw_cat(fb, x, y, w, h, &self.analyzer.spectrum_levels(), self.beat_pulse, self.cat_paw_left);
        }
    }
}

fn bass_treble(spectrum: &[f32]) -> (f32, f32) {
    if spectrum.is_empty() {
        return (0.0, 0.0);
    }
    let bass_bins = (spectrum.len() / 10).max(1);
    let treble_bins = (spectrum.len() / 8).max(1);
    let bass = spectrum[..bass_bins].iter().sum::<f32>() / bass_bins as f32;
    let treble = spectrum[spectrum.len() - treble_bins..].iter().sum::<f32>() / treble_bins as f32;
    (bass.clamp(0.0, 1.0), treble.clamp(0.0, 1.0))
}

fn draw_spectrum(fb: &mut FrameBuffer, spectrum: &[f32], x: i32, y: i32, w: i32, h: i32) {
    let bar_w = (w / spectrum.len().max(1) as i32).max(1);
    for (i, level) in spectrum.iter().enumerate() {
        let bar_h = (level.clamp(0.0, 1.0) * h as f32) as u32;
        if bar_h == 0 {
            continue;
        }
        let bx = x + i as i32 * bar_w;
        Rectangle::new(Point::new(bx, y + h - bar_h as i32), Size::new((bar_w - 1).max(1) as u32, bar_h))
            .into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT))
            .draw(fb)
            .ok();
    }
}

fn draw_waveform(fb: &mut FrameBuffer, wave: &[f32], x: i32, y: i32, w: i32, h: i32) {
    let mid = y + h / 2;
    let mut prev: Option<Point> = None;
    for (i, sample) in wave.iter().enumerate() {
        let px = x + (i as i32 * w) / wave.len().max(1) as i32;
        let py = mid - (sample.clamp(-1.0, 1.0) * (h as f32 / 2.0)) as i32;
        let point = Point::new(px, py);
        if let Some(p0) = prev {
            draw_line(fb, p0, point, VISUALIZER_ACCENT);
        }
        prev = Some(point);
    }
}

fn draw_levels(fb: &mut FrameBuffer, peak: f32, rms: f32, x: i32, y: i32, w: i32) {
    let dim = MonoTextStyle::new(&SPLEEN_6X12, VISUALIZER_DIM);
    let bar_h = 30;
    for (i, (label, level)) in [("Peak", peak), ("RMS", rms)].into_iter().enumerate() {
        let by = y + i as i32 * 70;
        Text::new(label, Point::new(x, by - 6), dim).draw(fb).ok();
        Rectangle::new(Point::new(x, by), Size::new(w as u32, bar_h))
            .into_styled(PrimitiveStyle::with_stroke(VISUALIZER_SCENE_DIM, 1))
            .draw(fb)
            .ok();
        let fill_w = (level.clamp(0.0, 1.0) * w as f32) as u32;
        if fill_w > 0 {
            Rectangle::new(Point::new(x, by), Size::new(fill_w, bar_h))
                .into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT))
                .draw(fb)
                .ok();
        }
    }
}

/// Simple interpolated line for the oscilloscope trace -- same
/// approach as analyzer.rs's own (private) copy of this helper.
fn draw_line(fb: &mut FrameBuffer, p0: Point, p1: Point, color: Rgb565) {
    let (x0, y0, x1, y1) = (p0.x, p0.y, p1.x, p1.y);
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let steps = dx.max(dy).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x0 + ((x1 - x0) as f32 * t) as i32;
        let y = y0 + ((y1 - y0) as f32 * t) as i32;
        fb.draw_iter(core::iter::once(embedded_graphics::Pixel(Point::new(x, y), color))).ok();
    }
}

/// Bobs on bass energy, jabs both gloves outward on a beat hit.
fn draw_boxer(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, spectrum: &[f32], beat_pulse: f32) {
    let (bass, _) = bass_treble(spectrum);
    let dim = VISUALIZER_SCENE_DIM;
    let ground_y = y + h - 20;
    let cx = x + w / 2;
    let bob = (bass * 10.0) as i32;
    let punch = (beat_pulse * 46.0) as i32;

    Rectangle::new(Point::new(x, ground_y), Size::new(w as u32, 2)).into_styled(PrimitiveStyle::with_fill(dim)).draw(fb).ok();

    for leg_x in [cx - 22, cx + 8] {
        Rectangle::new(Point::new(leg_x, ground_y - 42 - bob), Size::new(14, 42))
            .into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT))
            .draw(fb)
            .ok();
    }
    Rectangle::new(Point::new(cx - 26, ground_y - 104 - bob), Size::new(52, 62))
        .into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT))
        .draw(fb)
        .ok();
    Rectangle::new(Point::new(cx - 14, ground_y - 132 - bob), Size::new(28, 28))
        .into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT))
        .draw(fb)
        .ok();
    Circle::new(Point::new(cx - 52 - punch, ground_y - 96 - bob), 22).into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT)).draw(fb).ok();
    Circle::new(Point::new(cx + 8 + punch, ground_y - 96 - bob), 22).into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT)).draw(fb).ok();

    let label = MonoTextStyle::new(&SPLEEN_6X12, dim);
    Text::new("BOXER -- BASS+BEAT", Point::new(x, y + 12), label).draw(fb).ok();
}

/// Drives left-to-right on a loop, hops on a beat, headlight/rear
/// glow trail brighten with treble energy.
fn draw_car(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, spectrum: &[f32], beat_pulse: f32, car_x: f32) {
    let (_, treble) = bass_treble(spectrum);
    let dim = VISUALIZER_SCENE_DIM;
    let ground_y = y + h - 40;
    let car_w = 130;
    let cx = x + (car_x * (w - car_w) as f32) as i32;
    let cy = ground_y - 74 - (beat_pulse * 14.0) as i32;

    Rectangle::new(Point::new(x, ground_y), Size::new(w as u32, 2)).into_styled(PrimitiveStyle::with_fill(dim)).draw(fb).ok();

    let trail_w = (treble * 60.0) as u32;
    if trail_w > 0 {
        Rectangle::new(Point::new(cx - trail_w as i32, cy + 12), Size::new(trail_w, 4))
            .into_styled(PrimitiveStyle::with_fill(dim))
            .draw(fb)
            .ok();
    }
    Rectangle::new(Point::new(cx, cy), Size::new(car_w as u32, 34)).into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT)).draw(fb).ok();
    Rectangle::new(Point::new(cx + 24, cy - 16), Size::new(76, 18)).into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT)).draw(fb).ok();

    Circle::new(Point::new(cx + car_w - 12, cy + 8), 10).into_styled(PrimitiveStyle::with_fill(VISUALIZER_HEADLIGHT)).draw(fb).ok();

    for wheel_x in [cx + 18, cx + car_w - 34] {
        Circle::new(Point::new(wheel_x, cy + 26), 18).into_styled(PrimitiveStyle::with_stroke(VISUALIZER_ACCENT, 3)).draw(fb).ok();
    }

    let label = MonoTextStyle::new(&SPLEEN_6X12, dim);
    Text::new("CAR -- TREBLE+BEAT", Point::new(x, y + 12), label).draw(fb).ok();
}

/// A maneki-neko DJ, procedural pixel art in the same block style as
/// Boxer/Car (see the module doc comment). Head/ears nod and the
/// raised paw swings to `paw_left`'s side on `beat_pulse` -- flipped
/// once per beat onset in `tick`, so a real 4/4 reads as a steady
/// metronome swing. The boombox shows a tiny live spectrum readout,
/// same numbers `draw_spectrum` uses.
fn draw_cat(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, spectrum: &[f32], beat_pulse: f32, paw_left: bool) {
    let dim = VISUALIZER_SCENE_DIM;
    let white = Rgb565::WHITE;
    let black = Rgb565::BLACK;
    let red = Rgb565::new(22, 4, 2);
    let gold = VISUALIZER_HEADLIGHT;
    let boombox_body = Rgb565::new(3, 6, 3);

    let ground_y = y + h - 14;
    let cx = x + w / 2;
    let bob = (beat_pulse * 6.0) as i32;
    let paw_swing = ((beat_pulse * 16.0) as i32) * if paw_left { -1 } else { 1 };

    Rectangle::new(Point::new(x, ground_y), Size::new(w as u32, 2)).into_styled(PrimitiveStyle::with_fill(dim)).draw(fb).ok();

    // Body, then head on top of it (nods on the beat).
    Rectangle::new(Point::new(cx - 34, ground_y - 64), Size::new(68, 64)).into_styled(PrimitiveStyle::with_fill(white)).draw(fb).ok();
    Rectangle::new(Point::new(cx - 28, ground_y - 108 - bob), Size::new(56, 48)).into_styled(PrimitiveStyle::with_fill(white)).draw(fb).ok();

    // Ear patches (orange), headphone band + cups over them.
    Rectangle::new(Point::new(cx - 26, ground_y - 116 - bob), Size::new(14, 14)).into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT)).draw(fb).ok();
    Rectangle::new(Point::new(cx + 12, ground_y - 116 - bob), Size::new(14, 14)).into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT)).draw(fb).ok();
    Rectangle::new(Point::new(cx - 18, ground_y - 126 - bob), Size::new(36, 4)).into_styled(PrimitiveStyle::with_fill(black)).draw(fb).ok();
    for ear_x in [cx - 32, cx + 18] {
        Circle::new(Point::new(ear_x, ground_y - 118 - bob), 14).into_styled(PrimitiveStyle::with_fill(black)).draw(fb).ok();
        Circle::new(Point::new(ear_x + 3, ground_y - 115 - bob), 8).into_styled(PrimitiveStyle::with_fill(red)).draw(fb).ok();
    }

    // Eyes.
    for eye_x in [cx - 14, cx + 8] {
        Rectangle::new(Point::new(eye_x, ground_y - 92 - bob), Size::new(6, 8)).into_styled(PrimitiveStyle::with_fill(black)).draw(fb).ok();
    }

    // Collar + bell.
    Rectangle::new(Point::new(cx - 30, ground_y - 60), Size::new(60, 6)).into_styled(PrimitiveStyle::with_fill(red)).draw(fb).ok();
    Circle::new(Point::new(cx - 6, ground_y - 58), 10).into_styled(PrimitiveStyle::with_fill(gold)).draw(fb).ok();

    // Raised paw, swinging with the beat.
    Rectangle::new(Point::new(cx - 42 + paw_swing, ground_y - 118 - bob), Size::new(12, 26))
        .into_styled(PrimitiveStyle::with_fill(white))
        .draw(fb)
        .ok();
    Rectangle::new(Point::new(cx - 42 + paw_swing, ground_y - 118 - bob), Size::new(12, 6))
        .into_styled(PrimitiveStyle::with_fill(red))
        .draw(fb)
        .ok();

    // Boombox + a tiny live spectrum readout on top of it.
    let box_y = ground_y - 28;
    Rectangle::new(Point::new(cx - 44, box_y), Size::new(88, 26)).into_styled(PrimitiveStyle::with_fill(boombox_body)).draw(fb).ok();
    for spk_x in [cx - 32, cx + 16] {
        Circle::new(Point::new(spk_x, box_y + 5), 16).into_styled(PrimitiveStyle::with_stroke(dim, 2)).draw(fb).ok();
    }
    let bars = spectrum.len().min(8).max(1);
    let bar_w = (80 / bars as i32).max(1);
    for i in 0..bars {
        let bin = i * spectrum.len() / bars;
        let level = spectrum.get(bin).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let bar_h = (level * 14.0) as u32;
        if bar_h == 0 {
            continue;
        }
        Rectangle::new(Point::new(cx - 40 + i as i32 * bar_w, box_y - bar_h as i32), Size::new((bar_w - 1).max(1) as u32, bar_h))
            .into_styled(PrimitiveStyle::with_fill(VISUALIZER_ACCENT))
            .draw(fb)
            .ok();
    }

    let label = MonoTextStyle::new(&SPLEEN_6X12, dim);
    Text::new("CAT -- BEAT+SPECTRUM", Point::new(x, y + 12), label).draw(fb).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_row_cycles_without_touching_analyzer_rows() {
        let mut app = VisualizerApp::new();
        // Navigate to row 3 (Scene) -- 5 rows total, starts at 0.
        for _ in 0..3 {
            app.tick(&Input { knob1: 1, ..Default::default() });
        }
        assert_eq!(app.list.selected, 3);
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.scene, 1, "editing the Scene row should cycle Off -> Boxer");
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.scene, 2, "-> Car");
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.scene, 3, "-> Cat");
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.scene, 0, "wraps back to Off");
    }

    /// The Cat scene's whole point: its raised paw swings side to
    /// side in time with a real beat -- driven by the same rising-
    /// edge-on-peak-level bookkeeping Boxer/Car's `beat_pulse` uses,
    /// so every beat onset must flip `cat_paw_left`.
    #[test]
    fn cat_paw_flips_side_on_every_beat_onset() {
        let mut app = VisualizerApp::new();
        // `level_meters()` only updates once real audio has actually
        // run through the wrapped `AnalyzerApp`'s processor -- `tick`
        // alone never touches it, so drive both together, same as the
        // real registry does every frame.
        let mut proc = app.audio_processor().expect("AnalyzerApp always has a processor");
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut flips = 0;
        let mut prev_side = app.cat_paw_left;
        for _ in 0..500 {
            proc.process(&mut buffer, 2, 48000.0);
            app.tick(&Input::default());
            if app.cat_paw_left != prev_side {
                flips += 1;
                assert!(app.beat_pulse > 0.9, "the paw must only flip on a real beat onset (beat_pulse freshly spiked to 1.0), not on some other frame");
                prev_side = app.cat_paw_left;
            }
        }
        assert!(flips > 0, "expected the demo signal to cross the beat threshold at least once over 500 audio blocks");
    }

    /// The whole point of `monitor` (see the module doc comment's
    /// "Monitor" section): the wrapped `AnalyzerApp` always emits its
    /// demo signal, but this app must stay silent in the real mix
    /// until you explicitly turn Monitor on -- otherwise the sim
    /// would hum a sine tone the instant it boots.
    #[test]
    fn audio_is_silent_until_monitor_is_turned_on() {
        let mut app = VisualizerApp::new();
        let mut proc = app.audio_processor().expect("AnalyzerApp always has a processor");
        let mut buffer = vec![1.0f32; 512 * 2];
        proc.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0), "Monitor defaults to Off -- no audible output until turned on");

        // Navigate to row 4 (Monitor) and turn it on.
        for _ in 0..4 {
            app.tick(&Input { knob1: 1, ..Default::default() });
        }
        assert_eq!(app.list.selected, 4);
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert!(app.monitor.load(Ordering::Relaxed), "editing the Monitor row should turn it on");

        // The processor built from a fresh `audio_processor()` call
        // shares the same `Arc<AtomicBool>`, so re-request it the way
        // the real registry only ever calls this once -- instead
        // build a second processor sharing the app's now-true flag to
        // confirm the gate actually opens.
        let mut proc2 = VisualizerProcessor { inner: app.analyzer.audio_processor().unwrap(), monitor: Arc::clone(&app.monitor) };
        let mut buffer2 = vec![0.0f32; 512 * 2];
        proc2.process(&mut buffer2, 2, 48000.0);
        assert!(buffer2.iter().any(|&s| s != 0.0), "Monitor On should let the real demo signal through");
    }

    #[test]
    fn editing_mode_row_forwards_to_the_wrapped_analyzer() {
        let mut app = VisualizerApp::new();
        // Row 0 is already Mode -- edit it directly.
        let (before, _) = app.analyzer.mode_kind();
        app.tick(&Input { knob2: 1, ..Default::default() });
        let (after, _) = app.analyzer.mode_kind();
        assert_ne!(before, after, "editing row 0 (Mode) should change the wrapped AnalyzerApp's real mode");
    }

    #[test]
    fn draw_never_panics_in_any_scene() {
        let mut app = VisualizerApp::new();
        let mut fb = FrameBuffer::new();
        for scene in 0..4 {
            app.scene = scene;
            app.draw(&mut fb);
        }
    }
}
