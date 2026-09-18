//! A basic 4-track looper/recorder -- the DAW-basics app: record any
//! other app's live output (via AudioBus, same tap Clouds/Prism use
//! to granulate another app) into a track, layer more tracks on top,
//! and it all plays back in a loop. Nothing fancy on purpose:
//!
//! - No timeline/waveform editing -- a track is just "record it,
//!   play it back in a loop," the same workflow a hardware looper
//!   pedal (Boss RC-series, TC Ditto X4, etc.) uses, not a
//!   multitrack timeline editor.
//! - The **first** track ever recorded sets the loop length for the
//!   whole session -- every track recorded after that is
//!   automatically wrapped to that same length (classic looper
//!   behavior), rather than each track having its own independent
//!   length.
//! - Recording always replaces a track's previous content outright;
//!   there's no overdub-within-a-track or undo history.
//! - Mono only, same as every other app's internal signal path in
//!   this build.
//!
//! Registers its own mixed output on AudioBus/MixerBus/ModBus too,
//! same as every audio-producing app, so what you've looped can
//! itself be tapped, granulated, or leveled in the Mixer like
//! anything else.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_TRACKS: usize = 4;
/// Safety cap on the very first (freely-growing, since no loop
/// length exists yet) recording -- without this, holding Record on
/// an empty session forever would grow that track's buffer without
/// bound.
const MAX_LOOP_SECONDS: f32 = 60.0;

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Running,
    ClearAll,
    TrackInput(usize),
    TrackRecord(usize),
    TrackVolume(usize),
    TrackMute(usize),
    TrackClear(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 1 + NUM_TRACKS; // Global, then one per track

struct TrackParams {
    /// Which AudioBus source this track records from -- one source
    /// at a time, not a mix (unlike Prism/Clouds's input *mixer*),
    /// since a recorder captures one thing per take.
    input_source: AtomicUsize,
    /// UI-toggled; the audio thread just reacts to its current value
    /// each block (see `TapeProcessor::process`'s edge detection).
    recording: AtomicBool,
    has_content: AtomicBool,
    mute: AtomicBool,
    volume: AtomicF32,
    /// How many samples long this track's take is -- 0 until it has
    /// ever finished a recording. Purely for the UI's "Xs recorded"
    /// display; matches `loop_length_samples` once that's set.
    length_samples: AtomicUsize,
    /// One-shot "wipe this track" pulse, set by the UI and consumed
    /// by the audio thread -- same convention as Sequencer's
    /// `audition_pending`, needed because the actual sample buffer
    /// lives audio-thread-side (see `TapeProcessor::track_bufs`) and
    /// the UI has no direct access to clear it itself.
    clear_request: AtomicBool,
}

impl TrackParams {
    fn new() -> Self {
        Self {
            input_source: AtomicUsize::new(0),
            recording: AtomicBool::new(false),
            has_content: AtomicBool::new(false),
            mute: AtomicBool::new(false),
            volume: AtomicF32::new(0.8),
            length_samples: AtomicUsize::new(0),
            clear_request: AtomicBool::new(false),
        }
    }
}

struct Params {
    tracks: [TrackParams; NUM_TRACKS],
    /// The transport: false (default -- every app starts stopped,
    /// see main.rs's own module doc comment) until explicitly
    /// started. Pausing again freezes the shared position exactly
    /// where it is -- playback holds silent, and recording holds off
    /// too, rather than silently overwriting part of a track while
    /// nothing seems to be happening on screen. Wired to the F3
    /// hardware button via `App::running`/`toggle_running`, same
    /// convention as Sequencer/Bloom/Madness/Nebula's own transports.
    running: AtomicBool,
    /// 0 until the first-ever recording finishes, then fixed for the
    /// rest of the session (until `clear_all_request`) -- see the
    /// module doc comment.
    loop_length_samples: AtomicUsize,
    /// The shared playback/record position, wrapping at
    /// `loop_length_samples` once that's set -- purely for the UI's
    /// progress bar; the audio thread keeps its own copy for the
    /// actual read/write indexing (see `TapeProcessor`).
    play_pos: AtomicUsize,
    clear_all_request: AtomicBool,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Tape", modbus);
        Self {
            tracks: std::array::from_fn(|_| TrackParams::new()),
            running: AtomicBool::new(false),
            loop_length_samples: AtomicUsize::new(0),
            play_pos: AtomicUsize::new(0),
            clear_all_request: AtomicBool::new(false),
            bus_out: audio_bus.register("Tape"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct TapeApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    audio_bus: Arc<AudioBus>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

impl TapeApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self { params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)), sensitivity, nav_speed, audio_bus, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            vec![Selection::Running, Selection::ClearAll]
        } else {
            let t = g - 1;
            vec![Selection::TrackInput(t), Selection::TrackRecord(t), Selection::TrackVolume(t), Selection::TrackMute(t), Selection::TrackClear(t)]
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

    fn group_name(&self, g: usize) -> String {
        if g == 0 { "Global".into() } else { format!("Track {g}") }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            let len = self.params.loop_length_samples.load(Ordering::Relaxed);
            if len == 0 { "no loop set yet".into() } else { format!("loop: {:.1}s", len as f32 / 48000.0) }
        } else {
            let t = g - 1;
            self.track_status(t)
        }
    }

    fn track_status(&self, t: usize) -> String {
        if self.params.tracks[t].recording.load(Ordering::Relaxed) {
            "RECORDING".into()
        } else if self.params.tracks[t].has_content.load(Ordering::Relaxed) {
            let secs = self.params.tracks[t].length_samples.load(Ordering::Relaxed) as f32 / 48000.0;
            if self.params.tracks[t].mute.load(Ordering::Relaxed) { format!("muted, {secs:.1}s") } else { format!("playing, {secs:.1}s") }
        } else {
            "empty".into()
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Running => "Running".into(),
            Selection::ClearAll => "Clear All".into(),
            Selection::TrackInput(_) => "Input".into(),
            Selection::TrackRecord(_) => "Record".into(),
            Selection::TrackVolume(_) => "Volume".into(),
            Selection::TrackMute(_) => "Mute".into(),
            Selection::TrackClear(_) => "Clear".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Running => {
                if self.params.running.load(Ordering::Relaxed) { "running".into() } else { "paused".into() }
            }
            Selection::ClearAll => "press knob2".into(),
            Selection::TrackInput(t) => {
                let idx = self.params.tracks[t].input_source.load(Ordering::Relaxed);
                self.audio_bus.names().get(idx).cloned().unwrap_or_else(|| "(none)".into())
            }
            Selection::TrackRecord(t) => self.track_status(t),
            Selection::TrackVolume(t) => format!("{:.2}", self.params.tracks[t].volume.get()),
            Selection::TrackMute(t) => {
                if self.params.tracks[t].mute.load(Ordering::Relaxed) { "MUTED".into() } else { "on".into() }
            }
            Selection::TrackClear(_) => "press knob2".into(),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::ClearAll | Selection::TrackRecord(_) | Selection::TrackClear(_) => {} // press-only, see `reset`
            Selection::Running => self.params.running.store(delta > 0, Ordering::Relaxed),
            Selection::TrackInput(t) => {
                let num_inputs = self.audio_bus.len().max(1);
                let cur = self.params.tracks[t].input_source.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(num_inputs as i32);
                self.params.tracks[t].input_source.store(next as usize, Ordering::Relaxed);
            }
            Selection::TrackVolume(t) => bump(&self.params.tracks[t].volume, delta, sensitivity, 0.0, 1.5),
            Selection::TrackMute(t) => self.params.tracks[t].mute.store(delta > 0, Ordering::Relaxed),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Running => {} // no sensible single default -- same convention every other transport in this build uses
            Selection::ClearAll => self.params.clear_all_request.store(true, Ordering::Relaxed),
            Selection::TrackRecord(t) => {
                let cur = self.params.tracks[t].recording.load(Ordering::Relaxed);
                self.params.tracks[t].recording.store(!cur, Ordering::Relaxed);
            }
            Selection::TrackClear(t) => self.params.tracks[t].clear_request.store(true, Ordering::Relaxed),
            Selection::TrackVolume(t) => self.params.tracks[t].volume.set(0.8),
            Selection::TrackMute(t) => self.params.tracks[t].mute.store(false, Ordering::Relaxed),
            Selection::TrackInput(_) => {} // no single sensible default among equal choices
        }
    }
}

impl App for TapeApp {
    fn running(&self) -> Option<bool> {
        Some(self.params.running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let cur = self.params.running.load(Ordering::Relaxed);
        self.params.running.store(!cur, Ordering::Relaxed);
    }

    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate(input.knob1, rows.len(), self.nav_speed.get() as i32);
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
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(TapeProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            track_bufs: std::array::from_fn(|_| Vec::new()),
            prev_recording: [false; NUM_TRACKS],
            play_pos: 0,
            mono_buf: Vec::new(),
            live_scratch: std::array::from_fn(|_| Vec::new()),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("Tape", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(0, 63, 10));
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

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
        self.list.draw(fb, 16, 44, 24, 10, &display_rows);

        // --- Right: 4 track lanes, a playhead line, and a filled
        // bar per track showing how much of the loop has content. ---
        let loop_len = self.params.loop_length_samples.load(Ordering::Relaxed);
        let play_pos = self.params.play_pos.load(Ordering::Relaxed);
        let lane_x = 370;
        let lane_w = 230;
        let lane_h = 30;
        let gap = 12;
        let top = 70;

        for t in 0..NUM_TRACKS {
            let y = top + t as i32 * (lane_h + gap);
            let recording = self.params.tracks[t].recording.load(Ordering::Relaxed);
            let has_content = self.params.tracks[t].has_content.load(Ordering::Relaxed);
            let muted = self.params.tracks[t].mute.load(Ordering::Relaxed);
            let fill = if recording {
                Rgb565::new(30, 5, 5)
            } else if has_content && !muted {
                Rgb565::new(0, 35, 6)
            } else {
                Rgb565::new(3, 6, 3)
            };
            Rectangle::new(Point::new(lane_x, y), Size::new(lane_w as u32, lane_h as u32)).into_styled(PrimitiveStyle::with_fill(fill)).draw(fb).ok();
            Rectangle::new(Point::new(lane_x, y), Size::new(lane_w as u32, lane_h as u32))
                .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(10, 20, 10), 1))
                .draw(fb)
                .ok();

            if loop_len > 0 {
                let frac = (play_pos % loop_len) as f32 / loop_len as f32;
                let x = lane_x + (frac * lane_w as f32) as i32;
                Rectangle::new(Point::new(x, y), Size::new(2, lane_h as u32)).into_styled(PrimitiveStyle::with_fill(Rgb565::new(0, 63, 30))).draw(fb).ok();
            }

            Text::new(&format!("T{} {}", t + 1, self.track_status(t)), Point::new(lane_x + 6, y + lane_h - 8), accent).draw(fb).ok();
        }

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct TapeProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    /// The actual recorded audio -- audio-thread-only; the UI side
    /// only ever sees status atomics (see `TrackParams`), never this.
    track_bufs: [Vec<f32>; NUM_TRACKS],
    /// Local edge-detection for "recording just stopped," needed to
    /// finalize `loop_length_samples` exactly once per take rather
    /// than every block while a track sits idle.
    prev_recording: [bool; NUM_TRACKS],
    /// Audio-thread's own copy of the shared position -- `Params`'s
    /// copy is republished every block purely for the UI's playhead
    /// display.
    play_pos: usize,
    mono_buf: Vec<f32>,
    /// Per-block scratch holding each track's currently-selected
    /// live input source, refreshed once per block before the
    /// per-sample loop needs it.
    live_scratch: [Vec<f32>; NUM_TRACKS],
}

impl AudioProcessor for TapeProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);

        if self.params.clear_all_request.swap(false, Ordering::Relaxed) {
            self.params.loop_length_samples.store(0, Ordering::Relaxed);
            self.play_pos = 0;
            for t in 0..NUM_TRACKS {
                self.track_bufs[t].clear();
                self.params.tracks[t].has_content.store(false, Ordering::Relaxed);
                self.params.tracks[t].length_samples.store(0, Ordering::Relaxed);
                self.params.tracks[t].recording.store(false, Ordering::Relaxed);
                self.prev_recording[t] = false;
            }
        }
        for t in 0..NUM_TRACKS {
            if self.params.tracks[t].clear_request.swap(false, Ordering::Relaxed) {
                self.track_bufs[t].clear();
                self.params.tracks[t].has_content.store(false, Ordering::Relaxed);
                self.params.tracks[t].length_samples.store(0, Ordering::Relaxed);
            }
        }

        for t in 0..NUM_TRACKS {
            self.live_scratch[t].clear();
            self.live_scratch[t].resize(frames, 0.0);
            let src_idx = self.params.tracks[t].input_source.load(Ordering::Relaxed);
            if let Some(src) = self.audio_bus.get(src_idx) {
                let src = src.lock().unwrap();
                for (m, s) in self.live_scratch[t].iter_mut().zip(src.iter()) {
                    *m = *s;
                }
            }
        }

        let max_first_take_samples = (MAX_LOOP_SECONDS * sample_rate) as usize;

        // Paused: freeze exactly where things are -- no new samples
        // recorded, no playback position advancing, plain silence
        // out, same as a real tape machine's pause. Clearing above
        // still works regardless -- clearing a track shouldn't
        // require unpausing first. `buffer` is zeroed explicitly
        // here rather than relying on `mono_buf`'s already-zero state
        // plus whatever the caller pre-zeroed: MixBus does pre-zero
        // its scratch buffer before every call, but this must stay
        // correct when called directly too (every test in this file
        // does exactly that), not just through MixBus.
        if !self.params.running.load(Ordering::Relaxed) {
            for out in buffer.iter_mut() {
                *out = 0.0;
            }
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
            return;
        }

        for n in 0..frames {
            let mut loop_len = self.params.loop_length_samples.load(Ordering::Relaxed);

            for t in 0..NUM_TRACKS {
                let recording = self.params.tracks[t].recording.load(Ordering::Relaxed);
                let live_sample = self.live_scratch[t][n];

                if recording {
                    if loop_len == 0 {
                        if self.track_bufs[t].len() < max_first_take_samples {
                            self.track_bufs[t].push(live_sample);
                        }
                    } else {
                        if self.track_bufs[t].len() != loop_len {
                            self.track_bufs[t] = vec![0.0; loop_len];
                        }
                        let pos = self.play_pos % loop_len;
                        self.track_bufs[t][pos] = live_sample;
                    }
                }

                // Falling edge: this track's take just finished.
                if self.prev_recording[t] && !recording {
                    let recorded_len = self.track_bufs[t].len();
                    self.params.tracks[t].has_content.store(recorded_len > 0, Ordering::Relaxed);
                    self.params.tracks[t].length_samples.store(recorded_len, Ordering::Relaxed);
                    if loop_len == 0 && recorded_len > 0 {
                        // The first-ever take in this session sets
                        // the loop length for everyone from now on.
                        self.params.loop_length_samples.store(recorded_len, Ordering::Relaxed);
                        loop_len = recorded_len;
                        self.play_pos = 0;
                    }
                }
                self.prev_recording[t] = recording;
            }

            // Mix: a track contributes its recorded content while
            // playing back (has content, not muted, not currently
            // recording -- you monitor the *live* input while
            // recording instead, immediately below), or its live
            // input while actively recording (so you can hear what
            // you're laying down).
            let mut sample = 0.0f32;
            let mut active = 0usize;
            for t in 0..NUM_TRACKS {
                let recording = self.params.tracks[t].recording.load(Ordering::Relaxed);
                let volume = self.params.tracks[t].volume.get();
                if recording {
                    sample += self.live_scratch[t][n] * volume;
                    active += 1;
                } else if self.params.tracks[t].has_content.load(Ordering::Relaxed) && !self.params.tracks[t].mute.load(Ordering::Relaxed) && loop_len > 0 {
                    let pos = self.play_pos % loop_len;
                    if let Some(&s) = self.track_bufs[t].get(pos) {
                        sample += s * volume;
                        active += 1;
                    }
                }
            }
            // Same active-track headroom normalization every other
            // multi-voice app in this build uses -- layering more
            // tracks shouldn't itself make the mix louder.
            self.mono_buf[n] = sample / active.max(1) as f32;

            self.play_pos += 1;
            self.params.play_pos.store(self.play_pos, Ordering::Relaxed);
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

    fn new_processor(params: Arc<Params>, audio_bus: Arc<AudioBus>) -> TapeProcessor {
        TapeProcessor {
            params,
            audio_bus,
            track_bufs: std::array::from_fn(|_| Vec::new()),
            prev_recording: [false; NUM_TRACKS],
            play_pos: 0,
            mono_buf: Vec::new(),
            live_scratch: std::array::from_fn(|_| Vec::new()),
        }
    }

    /// Pausing (the F3-wired transport) must freeze the playhead and
    /// silence the output, without discarding any recorded content
    /// -- unpausing should pick back up exactly where it left off.
    #[test]
    fn pausing_freezes_playhead_and_silences_output() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed); // production default is stopped; tests exercise playback/recording
        params.tracks[0].input_source.store(0, Ordering::Relaxed);
        params.tracks[0].volume.set(1.0);
        params.tracks[0].recording.store(true, Ordering::Relaxed);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 1.0);
        }
        proc.process(&mut buffer, 2, 48000.0);
        params.tracks[0].recording.store(false, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);
        let loop_len = params.loop_length_samples.load(Ordering::Relaxed);
        assert_eq!(loop_len, frames);

        params.running.store(false, Ordering::Relaxed);
        let pos_before = proc.play_pos;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            let peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
            assert_eq!(peak, 0.0, "paused playback should be silent");
        }
        assert_eq!(proc.play_pos, pos_before, "the playhead must not advance while paused");

        params.running.store(true, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);
        let peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak > 0.0, "unpausing should resume audible playback");
    }

    /// Recording a track and playing it back must reproduce exactly
    /// what was fed in -- the whole point of a recorder.
    #[test]
    fn records_and_plays_back_exactly() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed); // production default is stopped; tests exercise playback/recording
        params.tracks[0].input_source.store(0, Ordering::Relaxed);
        params.tracks[0].volume.set(1.0); // isolate playback content from the (separately-tested) volume knob
        params.tracks[0].recording.store(true, Ordering::Relaxed);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];
        let pattern = [0.5f32, -0.5, 0.25, -0.25, 0.1, -0.1, 0.75, -0.75];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            s.copy_from_slice(&pattern);
        }
        proc.process(&mut buffer, 2, 48000.0);
        params.tracks[0].recording.store(false, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0); // one more block to let the falling edge finalize

        assert_eq!(params.loop_length_samples.load(Ordering::Relaxed), frames, "the first take should set the loop length to exactly what was recorded");
        assert_eq!(proc.track_bufs[0], pattern, "the recorded buffer should exactly match the input");

        {
            let mut s = src.lock().unwrap();
            s.iter_mut().for_each(|v| *v = 0.0); // silence the input -- playback shouldn't need it
        }
        proc.process(&mut buffer, 2, 48000.0);
        let played: Vec<f32> = buffer.chunks(2).map(|f| f[0]).collect();
        for (i, (&expected, &got)) in pattern.iter().zip(played.iter()).enumerate() {
            assert!((expected - got).abs() < 1e-4, "sample {i}: expected {expected}, got {got}");
        }
    }

    /// A second track recorded after the loop length is established
    /// must be wrapped to that same length, not keep its own.
    #[test]
    fn second_track_wraps_to_established_loop_length() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src0 = audio_bus.register("source-0");
        let src1 = audio_bus.register("source-1");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed); // production default is stopped; tests exercise playback/recording
        params.tracks[1].input_source.store(1, Ordering::Relaxed);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];

        // Track 0 records first, setting the loop length to 8 samples.
        params.tracks[0].recording.store(true, Ordering::Relaxed);
        {
            let mut s = src0.lock().unwrap();
            s.resize(frames, 1.0);
        }
        proc.process(&mut buffer, 2, 48000.0);
        params.tracks[0].recording.store(false, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);
        assert_eq!(params.loop_length_samples.load(Ordering::Relaxed), 8);

        // Track 1 now records for *2 full loops* (16 samples) worth
        // of blocks -- its buffer must still end up exactly 8 long.
        params.tracks[1].recording.store(true, Ordering::Relaxed);
        {
            let mut s = src1.lock().unwrap();
            s.resize(frames, 0.5);
        }
        proc.process(&mut buffer, 2, 48000.0);
        proc.process(&mut buffer, 2, 48000.0);
        params.tracks[1].recording.store(false, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);

        assert_eq!(proc.track_bufs[1].len(), 8, "track 1 should be wrapped to the established 8-sample loop length, not its own 24");
    }

    /// Clear All must wipe every track and the established loop
    /// length, letting a new first take set a fresh one.
    #[test]
    fn clear_all_resets_everything() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed); // production default is stopped; tests exercise playback/recording
        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 1.0);
        }

        params.tracks[0].recording.store(true, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);
        params.tracks[0].recording.store(false, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);
        assert!(params.tracks[0].has_content.load(Ordering::Relaxed));

        params.clear_all_request.store(true, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);

        assert_eq!(params.loop_length_samples.load(Ordering::Relaxed), 0);
        assert!(!params.tracks[0].has_content.load(Ordering::Relaxed));
        assert!(proc.track_bufs[0].is_empty());
    }

    /// Several tracks playing at once must not sum to an amplitude
    /// that scales with how many are active -- same headroom
    /// regression class Bloom's voice pool hit.
    #[test]
    fn multiple_playing_tracks_stay_headroom_normalized() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let srcs: Vec<_> = (0..NUM_TRACKS).map(|i| audio_bus.register(format!("source-{i}"))).collect();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed); // production default is stopped; tests exercise playback/recording
        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];

        for (t, src) in srcs.iter().enumerate() {
            params.tracks[t].input_source.store(t, Ordering::Relaxed);
            params.tracks[t].volume.set(1.0);
            {
                let mut s = src.lock().unwrap();
                s.resize(frames, 1.0);
            }
            params.tracks[t].recording.store(true, Ordering::Relaxed);
            proc.process(&mut buffer, 2, 48000.0);
            params.tracks[t].recording.store(false, Ordering::Relaxed);
            proc.process(&mut buffer, 2, 48000.0);
        }

        for src in &srcs {
            src.lock().unwrap().iter_mut().for_each(|v| *v = 0.0);
        }
        proc.process(&mut buffer, 2, 48000.0);
        let peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak < 1.5, "peak grew with how many tracks were playing instead of staying headroom-normalized: {peak}");
    }

    /// The Mixer app's channel fader must only affect what reaches
    /// the device output, not what this app publishes to audio_bus.rs.
    #[test]
    fn mixer_fader_does_not_affect_audio_bus_publish() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed); // production default is stopped; tests exercise playback/recording
        params.mix_level.set(0.0);
        params.tracks[0].recording.store(true, Ordering::Relaxed);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 1.0);
        }
        proc.process(&mut buffer, 2, 48000.0);

        let device_peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert_eq!(device_peak, 0.0, "device output should be silent with mix_level at 0");

        let bus_out = params.bus_out.lock().unwrap();
        let bus_peak = bus_out.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(bus_peak > 0.0, "audio_bus publish (the live-monitored recording signal) should be unaffected by the Mixer channel fader");
    }

    /// The freshly-constructed, untouched `Params` -- the exact state
    /// a brand new app instance starts in -- must default to
    /// stopped. With N apps all loaded (and someday possibly far
    /// more), an app that defaults to *running* is a real risk on
    /// its own, not just an inconvenience.
    #[test]
    fn defaults_to_stopped() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        assert!(!params.running.load(Ordering::Relaxed), "Tape must start stopped, not running");
    }
}
