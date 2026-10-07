//! Now Playing: one screen for what is making sound right now and how it is
//! wired.
//!
//! - **Sources** are everything that sends notes (sequencers, Bloom, Hum,
//!   Session...). Each row shows which instrument it plays and lights up
//!   while that instrument is sounding. Knob 2 sends it to a different
//!   instrument; pressing the row lists that instrument's own settings (its
//!   patch, filter, macros...) right under it, ready to turn.
//! - **Sounding now** lists every channel that is producing audio at this
//!   moment, with a live level, and knob 2 on a row moves its mixer fader.
//!
//! Nothing here is private to this app: routes are the note bus's own (so
//! Portal's Notes page and each app's "Plays" row move with them), settings
//! travel through the instrument-settings bridge, and faders are the
//! Mixer's. This is only a window onto them that can also reach in.

use crate::app::{App, Input};
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::mixer_bus::MixerBus;
use crate::note_bus::{NoteBus, INTERNAL, NONE};
use crate::paramlist::ParamList;
use crate::spleen_fonts::SPLEEN_6X12;
use crate::spleen_fonts::SPLEEN_8X16;
use crate::util::AtomicF32;
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, text::Text};
use std::sync::atomic::Ordering;
use std::sync::Arc;

// Own palette: dark slate with an amber "on air" accent.
const BG: Rgb565 = Rgb565::new(2, 5, 5);
const INK: Rgb565 = Rgb565::new(27, 56, 26);
const ACCENT: Rgb565 = Rgb565::new(31, 44, 3);
const DIM: Rgb565 = Rgb565::new(11, 24, 20);
const CHIP: Rgb565 = Rgb565::new(5, 13, 12);

/// A channel quieter than this is not "sounding".
const AUDIBLE: f32 = 0.002;
/// Fader steps per encoder click, in the Mixer's own units (0..1.5).
const FADER_STEP: f32 = 0.02;
const FADER_MAX: f32 = 1.5;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Header(usize),
    /// A note source, by its index on the note bus.
    Source(usize),
    /// Setting `i` of the instrument the open source plays.
    Setting { slot: usize, index: usize },
    /// An audio channel that is sounding, by its index on the audio bus.
    Sound(usize),
}

pub struct NowPlayingApp {
    notes: Arc<NoteBus>,
    audio: Arc<AudioBus>,
    mixer: Arc<MixerBus>,
    nav: Arc<AtomicF32>,
    list: ParamList,
    /// The source whose instrument settings are listed under it.
    open: Option<usize>,
}

impl NowPlayingApp {
    pub fn new(notes: Arc<NoteBus>, audio: Arc<AudioBus>, mixer: Arc<MixerBus>, nav: Arc<AtomicF32>) -> Self {
        Self { notes, audio, mixer, nav, list: ParamList::new(), open: None }
    }

    /// The loudest sample in a channel's last block (0 if it is busy being
    /// written, so the audio thread is never made to wait).
    fn peak(&self, idx: usize) -> f32 {
        let Some(buf) = self.audio.peek(idx) else { return 0.0 };
        let Ok(b) = buf.try_lock() else { return 0.0 };
        b.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    fn mixer_index(&self, name: &str) -> Option<usize> {
        self.mixer.names().iter().position(|n| n == name)
    }

    /// The instrument slot a route plays, if it is one.
    fn slot_of(route: usize) -> Option<usize> {
        (route != INTERNAL && route != NONE).then_some(route)
    }

    fn sounding(&self) -> Vec<usize> {
        (0..self.audio.len()).filter(|&i| self.peak(i) > AUDIBLE).collect()
    }

    fn rows(&self) -> Vec<Row> {
        let sources = self.notes.sources();
        let mut r = vec![Row::Header(0)];
        for (i, (_, _, route)) in sources.iter().enumerate() {
            r.push(Row::Source(i));
            if self.open == Some(i) {
                if let Some(slot) = Self::slot_of(route.load(Ordering::Relaxed)) {
                    r.extend((0..self.notes.instrument_settings(slot).len()).map(|index| Row::Setting { slot, index }));
                }
            }
        }
        r.push(Row::Header(1));
        r.extend(self.sounding().into_iter().map(Row::Sound));
        r
    }

    /// Whether the instrument at `route` has notes in it right now.
    fn playing(&self, route: usize) -> bool {
        Self::slot_of(route).and_then(|s| self.notes.inbox(s)).is_some_and(|i| i.busy())
    }

    fn meter(level: f32) -> String {
        let n = ((level.clamp(0.0, 1.0)).sqrt() * 10.0).round() as usize;
        format!("{}{}", "#".repeat(n), "-".repeat(10 - n))
    }

    fn row_text(&self, row: Row) -> (String, String, bool) {
        match row {
            Row::Header(0) => {
                let sources = self.notes.sources();
                let live = sources.iter().filter(|(_, _, r)| self.playing(r.load(Ordering::Relaxed))).count();
                (format!("NOTES  {} sources, {} sounding", sources.len(), live), String::new(), true)
            }
            Row::Header(_) => (format!("SOUNDING NOW  {}", self.sounding().len()), String::new(), true),
            Row::Source(i) => {
                let sources = self.notes.sources();
                let Some((name, _, route)) = sources.get(i) else { return (String::new(), String::new(), false) };
                let r = route.load(Ordering::Relaxed);
                let dot = if self.playing(r) { "*" } else { " " };
                let open = if self.open == Some(i) { "-" } else { "+" };
                (format!("{open} {name}"), format!("{dot} {}", self.notes.route_name(r)), false)
            }
            Row::Setting { slot, index } => {
                let s = self.notes.instrument_settings(slot);
                match s.get(index) {
                    Some(s) => (format!("    {}", s.label), s.value.clone(), false),
                    None => (String::new(), String::new(), false),
                }
            }
            Row::Sound(i) => {
                let name = self.audio.source_name(i);
                let fader = self.mixer_index(&name).and_then(|m| self.mixer.level(m)).map(|l| format!(" {:.0}%", l.get() * 100.0)).unwrap_or_default();
                (name, format!("{}{fader}", Self::meter(self.peak(i))), false)
            }
            Row::Header(_) => unreachable!(),
        }
    }

    fn edit(&mut self, row: Row, delta: i32, press: bool) {
        match row {
            Row::Source(i) => {
                if delta != 0 {
                    self.notes.step_source(i, delta);
                    // the settings under the row belong to the old instrument
                    if self.open == Some(i) {
                        self.open = None;
                    }
                } else if press {
                    self.open = if self.open == Some(i) { None } else { Some(i) };
                }
            }
            Row::Setting { slot, index } if delta != 0 => self.notes.adjust_instrument_setting(slot, index, delta),
            Row::Sound(i) => {
                let name = self.audio.source_name(i);
                if let Some(level) = self.mixer_index(&name).and_then(|m| self.mixer.level(m)) {
                    if press {
                        level.set(1.0);
                    } else if delta != 0 {
                        level.set((level.get() + delta.signum() as f32 * FADER_STEP).clamp(0.0, FADER_MAX));
                    }
                }
            }
            _ => {}
        }
    }

    fn menu_rows(&self) -> Vec<(String, String, bool)> {
        self.rows().into_iter().map(|r| self.row_text(r)).collect()
    }
}

impl App for NowPlayingApp {
    fn tick(&mut self, input: &Input) {
        let rows = self.rows();
        self.list.navigate_input(input, rows.len(), self.nav.get() as i32);
        let sel = self.list.selected.min(rows.len() - 1);
        self.edit(rows[sel], input.knob2, input.knob2_press || input.knob1_press);
    }

    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        Text::new("NOW PLAYING", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_8X16, ACCENT)).draw(f).ok();
        let rows: Vec<(String, String)> = self.menu_rows().into_iter().map(|(a, b, _)| (a, b)).collect();
        self.list.draw_themed(f, 16, 54, 24, 11, &rows, INK, DIM, CHIP);
        Text::new("knob 2: change what a source plays / a fader   press: open its settings", Point::new(16, 340), MonoTextStyle::new(&SPLEEN_6X12, DIM)).draw(f).ok();
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.menu_rows()
    }

    fn slint_selected(&self) -> usize {
        self.list.selected
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(NowPlayingApp::new(ctx.get(), ctx.get(), ctx.get(), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Setting;
    use crate::modbus::ModBus;

    fn world() -> (Arc<NoteBus>, Arc<AudioBus>, Arc<MixerBus>, NowPlayingApp, crate::note_bus::NoteOut) {
        let notes = Arc::new(NoteBus::new());
        let audio = Arc::new(AudioBus::new());
        let mixer = Arc::new(MixerBus::new());
        notes.declare_instrument("hydra", "Hydra");
        notes.declare_instrument("plaits", "Plaits");
        let out = notes.register_source("Sequencer");
        let app = NowPlayingApp::new(notes.clone(), audio.clone(), mixer.clone(), Arc::new(AtomicF32::new(1.0)));
        (notes, audio, mixer, app, out)
    }

    fn row_of(app: &NowPlayingApp, name: &str) -> Row {
        app.rows().into_iter().find(|r| app.row_text(*r).0.contains(name)).unwrap_or_else(|| panic!("no row {name}: {:?}", app.menu_rows()))
    }

    #[test]
    fn a_source_shows_which_instrument_it_plays_and_knob_2_changes_it() {
        let (notes, _a, _m, mut app, out) = world();
        let row = row_of(&app, "Sequencer");
        assert!(app.row_text(row).1.contains("Own sound"));
        app.edit(row, 1, false);
        let first = app.row_text(row).1;
        assert!(first.contains("None"), "{first}");
        app.edit(row, 1, false);
        assert!(app.row_text(row).1.contains("Hydra"));
        assert_eq!(notes.route_name(out.route()), "Hydra", "it is the bus's own route, the same one Portal and the app's Plays row show");
        app.edit(row, -1, false);
        assert!(app.row_text(row).1.contains("None"));
    }

    #[test]
    fn a_source_lights_up_while_its_instrument_has_notes_in_it() {
        let (notes, _a, _m, app, out) = world();
        out.set_route(notes.instrument_index("Hydra").unwrap());
        let row = row_of(&app, "Sequencer");
        assert!(!app.row_text(row).1.starts_with('*'));
        notes.inbox(notes.instrument_index("Hydra").unwrap()).unwrap().note_on(60, 100);
        assert!(app.row_text(row).1.starts_with('*'), "{}", app.row_text(row).1);
        assert!(app.row_text(Row::Header(0)).0.contains("1 sounding"));
    }

    #[test]
    fn pressing_a_source_lists_its_instruments_settings_and_turning_one_sends_the_edit() {
        let (notes, _a, _m, mut app, out) = world();
        let slot = notes.instrument_index("Hydra").unwrap();
        out.set_route(slot);
        notes.publish_settings(slot, vec![Setting { label: "Cutoff".into(), value: "8 kHz".into() }, Setting { label: "Preset".into(), value: "Super Saw".into() }]);
        let row = row_of(&app, "Sequencer");
        let before = app.rows().len();
        app.edit(row, 0, true);
        assert_eq!(app.rows().len(), before + 2, "two settings appear under the source");
        let cutoff = row_of(&app, "Cutoff");
        assert_eq!(app.row_text(cutoff).1, "8 kHz");
        app.edit(cutoff, 3, false);
        assert_eq!(notes.take_setting_edits(slot), vec![(0, 3)]);
        app.edit(row, 0, true);
        assert_eq!(app.rows().len(), before, "pressing again closes them");
    }

    #[test]
    fn choosing_another_instrument_closes_the_old_ones_settings() {
        let (notes, _a, _m, mut app, out) = world();
        let slot = notes.instrument_index("Hydra").unwrap();
        out.set_route(slot);
        notes.publish_settings(slot, vec![Setting { label: "Cutoff".into(), value: "1".into() }]);
        let row = row_of(&app, "Sequencer");
        app.edit(row, 0, true);
        app.edit(row, 1, false);
        assert!(app.rows().iter().all(|r| !matches!(r, Row::Setting { .. })));
    }

    #[test]
    fn only_channels_that_are_making_sound_are_listed_and_their_fader_moves() {
        let (_n, audio, mixer, mut app, _o) = world();
        let mods = ModBus::new();
        let loud = audio.register("Hydra");
        let _quiet = audio.register("Plaits");
        let (level, _) = mixer.register("Hydra", &mods);
        mixer.register("Plaits", &mods);
        *loud.lock().unwrap() = vec![0.0, 0.5, -0.4, 0.1];
        let sounding: Vec<Row> = app.rows().into_iter().filter(|r| matches!(r, Row::Sound(_))).collect();
        assert_eq!(sounding.len(), 1, "Plaits is silent");
        let text = app.row_text(sounding[0]);
        assert_eq!(text.0, "Hydra");
        assert!(text.1.contains('#') && text.1.contains("100%"), "{}", text.1);
        app.edit(sounding[0], 5, false);
        assert!((level.get() - 1.02).abs() < 1e-4, "the Mixer's own fader moved: {}", level.get());
        app.edit(sounding[0], 0, true);
        assert_eq!(level.get(), 1.0, "pressing puts it back");
        for _ in 0..100 {
            app.edit(sounding[0], 1, false);
        }
        assert_eq!(level.get(), FADER_MAX);
    }

    #[test]
    fn it_survives_a_world_with_nothing_in_it() {
        let app = NowPlayingApp::new(Arc::new(NoteBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()), Arc::new(AtomicF32::new(1.0)));
        assert_eq!(app.rows().len(), 2, "just the two headers");
        let mut app = app;
        app.tick(&Input::default());
    }

    #[test]
    fn it_draws() {
        let (_n, _a, _m, mut app, _o) = world();
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
