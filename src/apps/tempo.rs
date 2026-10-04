//! Tempo: the device transport's front panel. One tempo, one play
//! button, for everything that follows the device clock (Session,
//! Skins, the Looper) -- plus tap tempo, a metronome, and MIDI clock in
//! and out so the Portamax can lead or follow a drum machine, a DAW or
//! another Portamax.
//!
//! Nothing here keeps time itself; it's all `crate::clock`. This app is
//! the controls and the click.
//!
//! Controls: any pad taps the tempo (four taps or more average out),
//! SELECT / F3 start and stop, up/down picks a setting, left/right
//! changes it.

use crate::app::{App, Input, SlintExtra};
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Extra, Size2, Sound};
use crate::clock::{Clock, Snap, Source};
use crate::display::FrameBuffer;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

const NAME: &str = "Tempo";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Tempo,
    Source,
    SendClock,
    Meter,
    Click,
    ClickLevel,
}
const ROWS: [Row; 6] = [Row::Tempo, Row::Source, Row::SendClock, Row::Meter, Row::Click, Row::ClickLevel];

/// Metronome: off, on, or only while something's recording (the Looper
/// and Session raise `Clock::recording`).
const CLICK_NAMES: [&str; 3] = ["off", "on", "when recording"];

pub struct Shared {
    pub click: AtomicU8,
    pub click_level: crate::util::AtomicF32,
}

/// The click, on the audio thread: a short sine blip on every beat,
/// higher on the bar's first.
struct Metronome {
    s: Arc<Shared>,
    clock: Arc<Clock>,
    snap: Snap,
    fi: usize,
    last_beat: i64,
    epoch: u64,
    t: f32,
    hz: f32,
    ph: f32,
}

impl Metronome {
    fn audible(&self) -> bool {
        match self.s.click.load(Ordering::Relaxed) {
            1 => true,
            2 => self.clock.recording.load(Ordering::Relaxed),
            _ => false,
        }
    }
}

impl Extra for Metronome {
    fn block(&mut self, _frames: usize, _sr: f32) {
        self.snap = self.clock.snap();
        self.fi = 0;
        if self.snap.epoch != self.epoch {
            self.epoch = self.snap.epoch;
            // a restart clicks its first beat
            self.last_beat = -1;
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        if self.snap.running {
            let beat = self.snap.beat_at(self.fi, sr);
            let b = beat.floor() as i64;
            if b != self.last_beat {
                self.last_beat = b;
                if self.audible() {
                    let bar = self.clock.bar_beats() as i64;
                    self.hz = if b.rem_euclid(bar) == 0 { 1760.0 } else { 1320.0 };
                    self.t = 0.0;
                    self.ph = 0.0;
                }
            }
        }
        self.fi += 1;
        if self.t > 0.04 {
            return (0.0, 0.0);
        }
        self.t += 1.0 / sr;
        self.ph = (self.ph + self.hz / sr).fract();
        // 1 ms attack so it doesn't click *badly*, then ~8 ms decay
        let env = (self.t / 0.001).min(1.0) * (-self.t / 0.008).exp();
        let y = (self.ph * std::f32::consts::TAU).sin() * env * self.s.click_level.get() * 0.9;
        (y, y)
    }
}

pub struct Tempo {
    sound: Sound,
    clock: Arc<Clock>,
    pub s: Arc<Shared>,
    row: usize,
    taps: Vec<Instant>,
    prev: [bool; 16],
    flash: f32,
    last_seen_beat: i64,
}

impl Tempo {
    pub fn new(sound: Sound, clock: Arc<Clock>) -> Tempo {
        sound.set_reverb(0.0);
        Tempo {
            sound,
            clock,
            s: Arc::new(Shared { click: AtomicU8::new(0), click_level: crate::util::AtomicF32::new(0.6) }),
            row: 0,
            taps: Vec::new(),
            prev: [false; 16],
            flash: 0.0,
            last_seen_beat: -1,
        }
    }

    /// One tap at `now`. Taps more than two seconds apart start over.
    fn tap(&mut self, now: Instant) {
        if self.taps.last().is_some_and(|l| now.duration_since(*l).as_secs_f32() > 2.0) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() >= 2 {
            let span = self.taps.last().unwrap().duration_since(self.taps[0]).as_secs_f32();
            let bpm = 60.0 * (self.taps.len() - 1) as f32 / span.max(1e-3);
            // a tenth of a bpm is finer than anyone can tap
            self.clock.set_bpm((bpm * 10.0).round() / 10.0);
        }
    }

    fn value(&self, r: Row) -> String {
        match r {
            Row::Tempo => format!("{:.1} bpm", self.clock.bpm()),
            Row::Source => match self.clock.source() {
                Source::Internal => "internal".into(),
                Source::MidiIn => "MIDI clock in".into(),
            },
            Row::SendClock => if self.clock.send_midi.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Row::Meter => format!("{}/4", self.clock.bar_beats() as u32),
            Row::Click => CLICK_NAMES[self.s.click.load(Ordering::Relaxed) as usize % 3].into(),
            Row::ClickLevel => format!("{:.0}", self.s.click_level.get() * 100.0),
        }
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Tempo => "Tempo",
            Row::Source => "Follow",
            Row::SendClock => "Send MIDI clock",
            Row::Meter => "Bar",
            Row::Click => "Metronome",
            Row::ClickLevel => "Click level",
        }
    }

    fn edit(&mut self, d: i32) {
        match ROWS[self.row] {
            Row::Tempo => {
                // coarse steps in whole bpm; a tapped 97.3 lands on 98 / 97
                let b = self.clock.bpm();
                let n = if d > 0 { b.floor() + d as f32 } else { b.ceil() + d as f32 };
                self.clock.set_bpm(n);
            }
            Row::Source => self.clock.set_source(if self.clock.source() == Source::Internal { Source::MidiIn } else { Source::Internal }),
            Row::SendClock => {
                let v = !self.clock.send_midi.load(Ordering::Relaxed);
                self.clock.send_midi.store(v, Ordering::Relaxed);
            }
            Row::Meter => {
                let b = (self.clock.bar_beats() as i32 + d).clamp(1, 12);
                self.clock.beats_per_bar.store(b as u8, Ordering::Relaxed);
            }
            Row::Click => {
                let c = (self.s.click.load(Ordering::Relaxed) as i32 + d).rem_euclid(3);
                self.s.click.store(c as u8, Ordering::Relaxed);
            }
            Row::ClickLevel => self.s.click_level.set((self.s.click_level.get() + d as f32 * 0.05).clamp(0.0, 1.0)),
        }
    }

    fn beat_in_bar(&self) -> Option<(u32, f64)> {
        let s = self.clock.snap();
        s.running.then(|| {
            let beat = self.clock.beat_now();
            ((beat.floor() as i64).rem_euclid(self.clock.bar_beats() as i64) as u32, beat.fract())
        })
    }
}

impl App for Tempo {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.s.click.load(Ordering::Relaxed) != 0
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (Tempo::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn running(&self) -> Option<bool> {
        Some(self.clock.running())
    }
    fn toggle_running(&mut self) {
        self.clock.toggle();
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.clock.running() { "STOP" } else { "PLAY" })
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let lit = self.beat_in_bar();
        std::array::from_fn(|p| match lit {
            Some((b, f)) if f < 0.25 => {
                if b == 0 {
                    PadColor::Red
                } else {
                    PadColor::Yellow
                }
            }
            _ => {
                if p == 0 {
                    PadColor::Green
                } else {
                    PadColor::Off
                }
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        let now = Instant::now();
        if (0..16).any(|p| input.grid[p] && !self.prev[p]) {
            self.tap(now);
            self.flash = 1.0;
        }
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            self.edit(input.knob2.signum());
        }
        if input.knob1_press {
            self.toggle_running();
        }
        self.flash *= 0.85;
        if let Some((b, _)) = self.beat_in_bar() {
            if b as i64 != self.last_seen_beat {
                self.last_seen_beat = b as i64;
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::rgb(18, 22, 30);
        let panel = kit::rgb(32, 38, 52);
        let ink = kit::rgb(120, 220, 200);
        let dim = kit::rgb(120, 130, 150);
        let hot = kit::rgb(250, 120, 90);
        kit::round_rect(fb, 0, 0, 640, 360, 0, bg);
        kit::text(fb, NAME, 12, 8, Size2::Medium, kit::WHITE, -1);
        let st = match (self.clock.running(), self.clock.source()) {
            (true, Source::MidiIn) => "playing - following MIDI clock",
            (true, _) => "playing",
            (false, Source::MidiIn) => "waiting for MIDI start",
            (false, _) => "stopped",
        };
        kit::text(fb, st, 628, 12, Size2::Small, dim, 1);

        // The tempo, big, flashing on taps.
        kit::round_rect(fb, 12, 44, 300, 150, 12, kit::blend(panel, ink, self.flash * 0.35));
        let bpm = self.clock.bpm();
        kit::text(fb, &format!("{:.1}", bpm), 162, 78, Size2::Huge, kit::WHITE, 0);
        kit::text(fb, "BPM", 162, 150, Size2::Small, dim, 0);

        // The bar: one light per beat.
        let beats = self.clock.bar_beats() as i32;
        let lit = self.beat_in_bar();
        let w = (300 - (beats - 1) * 6) / beats;
        for b in 0..beats {
            let on = lit.is_some_and(|(cur, f)| cur as i32 == b && f < 0.5);
            let base = if b == 0 { hot } else { ink };
            kit::round_rect(fb, 12 + b * (w + 6), 206, w, 28, 6, if on { base } else { kit::blend(panel, base, 0.18) });
        }
        if let Some(_) = lit {
            let pos = self.clock.beat_now();
            let bar = (pos / self.clock.bar_beats()).floor() as i64 + 1;
            kit::text(fb, &format!("bar {bar}"), 162, 244, Size2::Small, dim, 0);
        }

        for (i, r) in ROWS.iter().enumerate() {
            let y = 44 + i as i32 * 34;
            let sel = i == self.row;
            kit::round_rect(fb, 326, y, 302, 28, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, Tempo::label(*r), 336, y + 8, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            kit::text(fb, &self.value(*r), 618, y + 8, Size2::Small, ink, 1);
        }
        kit::paragraph(fb, "Tap any pad in time to set the tempo. Session, Skins and the Looper all follow this clock.", 326, 254, 300, Size2::Small, dim);
        kit::footer(fb, "pads: tap   SELECT/F3: play/stop   up/down + left/right: settings", panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let m = Metronome { s: Arc::clone(&self.s), clock: Arc::clone(&self.clock), snap: self.clock.snap(), fi: 0, last_beat: -1, epoch: 0, t: 1.0, hz: 1320.0, ph: 0.0 };
        Some(self.sound.processor(None, Some(Box::new(m))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(Tempo::new(Sound::new(NAME, &modbus, &mixer, &bus), Clock::shared()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn tapping_sets_the_tempo() {
        let clock = Arc::new(Clock::new());
        let mut t = Tempo::new(Sound::detached(), Arc::clone(&clock));
        let t0 = Instant::now();
        for k in 0..5 {
            t.tap(t0 + Duration::from_millis(k * 600));
        }
        assert!((clock.bpm() - 100.0).abs() < 0.2, "{}", clock.bpm());
        // a pause starts a new count
        t.tap(t0 + Duration::from_millis(6000));
        t.tap(t0 + Duration::from_millis(6500));
        assert!((clock.bpm() - 120.0).abs() < 0.2, "{}", clock.bpm());
    }

    #[test]
    fn the_metronome_clicks_on_every_beat() {
        let clock = Arc::new(Clock::new());
        let mut t = Tempo::new(Sound::detached(), Arc::clone(&clock));
        t.s.click.store(1, Ordering::Relaxed);
        let mut p = t.audio_processor().unwrap();
        clock.set_bpm(120.0);
        clock.start();
        clock.end_block(1, 48_000.0);
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 480 * 2];
        for _ in 0..200 {
            buf.iter_mut().for_each(|x| *x = 0.0);
            p.process(&mut buf, 2, 48_000.0);
            out.extend(buf.chunks(2).map(|f| f[0]));
            clock.end_block(480, 48_000.0);
        }
        // onsets: first sample above a threshold after silence
        let mut onsets = vec![];
        let mut quiet = 3000;
        let peak = out.iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak > 0.1, "peak {peak}");
        for (i, x) in out.iter().enumerate() {
            if x.abs() > 0.01 && quiet > 2000 {
                onsets.push(i);
            }
            quiet = if x.abs() > 0.01 { 0 } else { quiet + 1 };
        }
        // 2 s at 120 bpm = 4 beats, half a second apart
        assert_eq!(onsets.len(), 4, "{onsets:?}");
        for w in onsets.windows(2) {
            assert!(((w[1] - w[0]) as i64 - 24_000).abs() < 100, "{onsets:?}");
        }
        assert!(onsets[0] < 100);
        // off: silence
        t.s.click.store(0, Ordering::Relaxed);
        buf.iter_mut().for_each(|x| *x = 0.0);
        for _ in 0..100 {
            p.process(&mut buf, 2, 48_000.0);
            clock.end_block(480, 48_000.0);
        }
        assert!(buf.iter().all(|x| x.abs() < 1e-4));
    }

    #[test]
    fn settings_reach_the_clock_and_it_draws() {
        let clock = Arc::new(Clock::new());
        let mut t = Tempo::new(Sound::detached(), Arc::clone(&clock));
        t.row = 0;
        t.edit(1);
        assert_eq!(clock.bpm(), 121.0);
        t.row = 1;
        t.edit(1);
        assert_eq!(clock.source(), Source::MidiIn);
        t.row = 2;
        t.edit(1);
        assert!(clock.send_midi.load(Ordering::Relaxed));
        t.row = 3;
        t.edit(-1);
        assert_eq!(clock.bar_beats(), 3.0);
        t.toggle_running();
        assert!(clock.running());
        let mut fb = FrameBuffer::new();
        t.draw(&mut fb);
    }
}
