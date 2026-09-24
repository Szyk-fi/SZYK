//! The master mixer: one channel strip per app that actually produces
//! audio (Plaits, Sequencer, Bloom, Madness, Clouds, Prism -- anything
//! that calls `mixer_bus.register` alongside its `audio_bus.register`,
//! discovered live the same way Clouds/Prism enumerate their own input
//! mixers, not a hardcoded list here), plus one final Master Volume
//! stage applied after all of them have summed together (see
//! audio.rs's `MixBus::process`). Reached via a dedicated hardware
//! button (F4, see os.rs) so it's always one press away regardless of
//! which app is on screen.
//!
//! Every channel fader is also a ModBus modulation target (see
//! mixer_bus.rs), same as every other continuous knob in this build --
//! Pam's can duck/swell any app's presence in the mix, not just its
//! own internal parameters.
//!
//! A channel's fader only affects what reaches the device output, not
//! what that app publishes to audio_bus.rs for another app (Clouds,
//! Prism) to tap -- so turning Bloom down in here doesn't also starve
//! Prism of signal if Prism happens to be granulating it.

use crate::app::{App, Input};
use crate::audio_bus::AudioBus;
use crate::display::{self, FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::Arc;

const MIN_LEVEL: f32 = 0.0;
const MAX_LEVEL: f32 = 1.5;
const DEFAULT_LEVEL: f32 = 1.0;

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(MIN_LEVEL, MAX_LEVEL);
    value.set(next);
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    MasterVolume,
    ChannelLevel(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 2;

pub struct MixerApp {
    master_volume: Arc<AtomicF32>,
    mixer_bus: Arc<MixerBus>,
    /// For the real live per-channel VU meters (matched to a mixer
    /// channel by name -- a channel registers into both buses under
    /// the same name, see mixer_bus.rs's own doc comment) -- not used
    /// for anything that affects the actual mix, only the visual.
    audio_bus: Arc<AudioBus>,
    /// List *browsing* speed still follows the shared Nav Speed
    /// setting, same as every other app -- only the fader *edit*
    /// sensitivity below is fixed, deliberately ignoring Settings'
    /// global `sensitivity` (continuous *musical* parameters), since a
    /// fader is a coarser, one-off "get it about right" control that
    /// should feel predictable regardless of how sensitivity is tuned
    /// elsewhere.
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Mixer's own palette: cool console blue-grey, not a device-wide
// theme -- the neutral, professional "control room" look of a real
// mixing console, deliberately understated next to every other app's
// stronger personality (this is the one screen meant to feel like
// plain infrastructure everything else passes through). ---

const MIXER_BG: Rgb565 = Rgb565::new(2, 6, 4);
const MIXER_TITLE: Rgb565 = Rgb565::new(25, 56, 30);
const MIXER_ACCENT: Rgb565 = Rgb565::new(9, 25, 17);
const MIXER_DIM: Rgb565 = Rgb565::new(13, 31, 16);
const MIXER_FADER_OUTLINE: Rgb565 = Rgb565::new(5, 10, 7);

impl MixerApp {
    pub fn new(master_volume: Arc<AtomicF32>, mixer_bus: Arc<MixerBus>, nav_speed: Arc<AtomicF32>, audio_bus: Arc<AudioBus>) -> Self {
        Self { master_volume, mixer_bus, nav_speed, audio_bus, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    /// This channel's real live peak amplitude this block (0..~1.5),
    /// matched by name against `audio_bus` -- `0.0` if this channel's
    /// app doesn't also publish to `audio_bus` (not every app that
    /// registers a mixer channel taps/publishes raw audio for other
    /// apps to read, e.g. a pure CV/gate generator).
    fn channel_live_level(&self, mixer_idx: usize) -> f32 {
        let Some(name) = self.mixer_bus.names().get(mixer_idx).cloned() else { return 0.0 };
        let names = self.audio_bus.names();
        let Some(audio_idx) = names.iter().position(|n| *n == name) else { return 0.0 };
        let Some(buf) = self.audio_bus.get(audio_idx) else { return 0.0 };
        let guard = buf.lock().unwrap();
        guard.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    /// Real per-channel (name, fader level 0..1.5, live peak 0..~1.5,
    /// modbus-modulation indicator) plus the master, for a bespoke
    /// mixer-strip panel -- see `App::slint_extra`.
    pub(crate) fn output_visual(&self) -> Vec<(String, f32, f32)> {
        (0..self.mixer_bus.len())
            .map(|i| {
                let name = self.mixer_bus.names().get(i).cloned().unwrap_or_else(|| format!("Ch{}", i + 1));
                let fader = self.mixer_bus.level(i).map(|l| l.get()).unwrap_or(DEFAULT_LEVEL).clamp(MIN_LEVEL, MAX_LEVEL);
                (name, fader, self.channel_live_level(i))
            })
            .collect()
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![Selection::MasterVolume],
            _ => (0..self.mixer_bus.len()).map(Selection::ChannelLevel).collect(),
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

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::MasterVolume => "Master Volume".into(),
            Selection::ChannelLevel(i) => self.mixer_bus.names().get(i).cloned().unwrap_or_else(|| format!("Channel {}", i + 1)),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::MasterVolume => format!("{:.0}%", self.master_volume.get().clamp(MIN_LEVEL, MAX_LEVEL) * 100.0),
            Selection::ChannelLevel(i) => {
                let level = self.mixer_bus.level(i).map(|l| l.get()).unwrap_or(DEFAULT_LEVEL);
                format!("{:.0}%", level.clamp(MIN_LEVEL, MAX_LEVEL) * 100.0)
            }
        }
    }

    fn group_name(&self, g: usize) -> &'static str {
        if g == 0 { "Master" } else { "Channels" }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            self.leaf_value(Selection::MasterVolume)
        } else {
            format!("{} apps", self.mixer_bus.len())
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        match sel {
            Selection::MasterVolume => bump(&self.master_volume, delta, 1.0),
            Selection::ChannelLevel(i) => {
                if let Some(level) = self.mixer_bus.level(i) {
                    bump(&level, delta, 1.0);
                }
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::MasterVolume => self.master_volume.set(DEFAULT_LEVEL),
            Selection::ChannelLevel(i) => {
                if let Some(level) = self.mixer_bus.level(i) {
                    level.set(DEFAULT_LEVEL);
                }
            }
        }
    }
}

impl MixerApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
            })
            .collect()
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- see `ParamList::centered_scroll_window`. Returns
    /// `(window, selected_index_in_window, has_more_above,
    /// has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// Real fader level (0..1, clamped) for whichever leaf row this
    /// is, or `None` for a group row -- lets a live screen draw an
    /// actual meter bar next to the row instead of just its printed
    /// percentage. Windowed the same way `windowed_rows` is, so the
    /// two stay index-aligned.
    pub(crate) fn windowed_levels(&mut self, visible: usize) -> Vec<Option<f32>> {
        let rows = self.visible_rows();
        if rows.is_empty() || visible == 0 {
            return Vec::new();
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        rows[start..end]
            .iter()
            .map(|row| match row {
                Row::Leaf(Selection::MasterVolume) => Some((self.master_volume.get() / MAX_LEVEL).clamp(0.0, 1.0)),
                Row::Leaf(Selection::ChannelLevel(i)) => {
                    self.mixer_bus.level(*i).map(|l| (l.get() / MAX_LEVEL).clamp(0.0, 1.0))
                }
                Row::Group(_) => None,
            })
            .collect()
    }
}

impl App for MixerApp {
    fn system_role(&self) -> Option<crate::app::SystemRole> { Some(crate::app::SystemRole::Mixer) }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }
    fn slint_levels(&mut self, visible: usize) -> Vec<Option<f32>> {
        self.windowed_levels(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Mixer(crate::app::MixerExtra {
            master: self.master_volume.get().clamp(MIN_LEVEL, MAX_LEVEL),
            channels: self.output_visual(),
        })
    }

    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
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

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(MIXER_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, MIXER_TITLE);
        Text::new("Mixer", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, MIXER_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, MIXER_DIM);

        // Same shared list widget (font, row height, selected-row chip,
        // scroll-if-it-doesn't-fit) every other app's menu already
        // uses -- this used to hand-roll its own smaller-font,
        // non-scrolling loop instead, the one place in the app that
        // didn't match.
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
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, MIXER_BG, MIXER_DIM, MIXER_ACCENT);

        // --- Right: a bank of small vertical faders -- Master (always
        // shown, fixed in place) plus a scrolling window of channels
        // that follows the current selection, so however many apps
        // have registered (not capped at whatever happened to fit
        // this panel's width), every one of them is still reachable
        // and visible, just not all simultaneously on a screen this
        // size -- same "scroll to keep the selection in view" idea
        // `ParamList`'s own vertical list already uses, applied
        // horizontally here since faders read left-to-right. ---
        // Taller/wider than before -- the list column now uses the
        // same font every other app's menu does (see above), which
        // freed up vertical room this panel wasn't using down to the
        // hint line (337): track_top+track_h+label used to stop at
        // 222, leaving well over 100px unused.
        let track_top = 60;
        let track_h = 235;
        let track_w = 26;
        let gap = 16;
        let stride = track_w + gap;
        let base_x = 400;
        let right_margin = 12;

        let mut draw_fader = |x: i32, level: f32, label: &str, lit: bool| {
            let color = if lit { MIXER_ACCENT } else { MIXER_FADER_OUTLINE };
            Rectangle::new(Point::new(x, track_top), Size::new(track_w as u32, track_h as u32))
                .into_styled(PrimitiveStyle::with_stroke(MIXER_FADER_OUTLINE, 1))
                .draw(fb)
                .ok();
            let fill_h = ((level / MAX_LEVEL).clamp(0.0, 1.0) * track_h as f32) as i32;
            Rectangle::new(Point::new(x + 1, track_top + track_h - fill_h), Size::new((track_w - 2) as u32, fill_h.max(0) as u32))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
            let unity_y = track_top + track_h - ((DEFAULT_LEVEL / MAX_LEVEL) * track_h as f32) as i32;
            Rectangle::new(Point::new(x - 2, unity_y), Size::new((track_w + 4) as u32, 1))
                .into_styled(PrimitiveStyle::with_fill(MIXER_ACCENT))
                .draw(fb)
                .ok();
            let short: String = label.chars().take(4).collect();
            Text::new(&short, Point::new(x - 2, track_top + track_h + 12), dim).draw(fb).ok();
        };

        let master_lit = matches!(rows.get(self.list.selected), Some(Row::Leaf(Selection::MasterVolume)));
        draw_fader(base_x, self.master_volume.get().clamp(MIN_LEVEL, MAX_LEVEL), "MST", master_lit);

        let names = self.mixer_bus.names();
        let n = names.len();
        let channel_x0 = base_x + stride; // right after Master's own fixed slot
        let available_w = (display::WIDTH as i32 - channel_x0 - right_margin).max(0);
        let visible_faders = (available_w / stride).max(1) as usize;

        let selected_channel = match rows.get(self.list.selected) {
            Some(Row::Leaf(Selection::ChannelLevel(i))) => Some(*i),
            _ => None,
        };
        let scroll = if n <= visible_faders {
            0
        } else {
            let sel = selected_channel.unwrap_or(0);
            sel.saturating_sub(visible_faders.saturating_sub(1)).min(n - visible_faders)
        };
        let end = (scroll + visible_faders).min(n);

        for (slot, i) in (scroll..end).enumerate() {
            let level = self.mixer_bus.level(i).map(|l| l.get()).unwrap_or(DEFAULT_LEVEL).clamp(MIN_LEVEL, MAX_LEVEL);
            let lit = selected_channel == Some(i);
            let x = channel_x0 + slot as i32 * stride;
            draw_fader(x, level, &names[i], lit);
        }
        // "there's more, scroll to it" indicators -- otherwise a
        // channel bank that doesn't all fit would just look like it
        // stops short, with no hint that anything's missing.
        if scroll > 0 {
            Text::new("<", Point::new(channel_x0 - stride + 4, track_top + track_h / 2), accent).draw(fb).ok();
        }
        if end < n {
            Text::new(">", Point::new(channel_x0 + visible_faders as i32 * stride + 2, track_top + track_h / 2), accent).draw(fb).ok();
        }

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset to 100%", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::FrameBuffer;
    use crate::modbus::ModBus;

    /// Every channel that registers into `MixerBus` must be reachable
    /// from the Channels group, not capped at whatever used to fit
    /// the fader panel's width -- the same class of bug the input
    /// mixers (Clouds/Singularity/Prism) had.
    #[test]
    fn every_registered_channel_is_reachable_past_the_old_visible_cap() {
        let master_volume = Arc::new(AtomicF32::new(1.0));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = ModBus::new();
        let mixer_bus = Arc::new(MixerBus::new());
        for i in 0..15 {
            mixer_bus.register(format!("App {i}"), &modbus);
        }
        let app = MixerApp::new(master_volume, mixer_bus, nav_speed, Arc::new(AudioBus::new()));

        let leaves = app.group_leaves(1);
        assert_eq!(leaves.len(), 15, "expected all 15 registered channels to be reachable, got {}", leaves.len());
        assert!(matches!(leaves[14], Selection::ChannelLevel(14)), "the last channel (past the old on-screen cap) must still be reachable");
    }

    /// With more channels than fit on screen at once, drawing must not
    /// panic, and selecting a channel scrolled off the initial view
    /// must actually bring its fader on screen (not just leave the
    /// bank showing the same first few channels forever).
    #[test]
    fn draws_without_panicking_and_scrolls_to_a_channel_past_the_visible_window() {
        let master_volume = Arc::new(AtomicF32::new(1.0));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = ModBus::new();
        let mixer_bus = Arc::new(MixerBus::new());
        for i in 0..15 {
            mixer_bus.register(format!("App {i}"), &modbus);
        }
        let mut app = MixerApp::new(master_volume, mixer_bus, nav_speed, Arc::new(AudioBus::new()));
        app.expanded[1] = true; // Channels group expanded, so its leaves are in visible_rows
        let rows = app.visible_rows();
        let last_channel_row = rows.iter().position(|r| matches!(r, Row::Leaf(Selection::ChannelLevel(14)))).expect("channel 14 should be a row");
        app.list.selected = last_channel_row;

        let mut fb = FrameBuffer::new();
        app.draw(&mut fb); // must not panic with 15 channels registered

        let lit = fb.buffer().iter().filter(|&&p| p != 0).count();
        assert!(lit > 500, "expected the fader bank (including the scrolled-to selected channel) to draw something substantial, got {lit} lit pixels");
    }
}
