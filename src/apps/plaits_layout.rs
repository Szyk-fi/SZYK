//! Hot-reloaded panel geometry for the Plaits screen, kept in
//! `apps/plaits/layout.toml` instead of hardcoded in plaits.rs so it can
//! be driven by the Figma mockup (see the "Portamax Sim UI" file's
//! `Layout/Menu`, `Layout/Panel`, `Layout/PianoRoll` marker rectangles) --
//! resize/move those in Figma, ask for a re-sync, and this file gets
//! regenerated from their bounding boxes.
//!
//! Deliberately dependency-free hot reload: `LayoutWatcher::poll` just
//! checks the file's mtime a couple of times a second and re-parses on
//! change, rather than pulling in a filesystem-notification crate for
//! something this infrequent.

use serde::Deserialize;
use std::time::{Duration, Instant, SystemTime};

const LAYOUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/apps/plaits/layout.toml");
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Deserialize, Clone, Copy)]
pub struct Layout {
    pub menu_x: i32,
    pub menu_y: i32,
    pub panel_x: i32,
    pub panel_y: i32,
    pub panel_w: i32,
    pub default_roll_height: i32,
}

impl Default for Layout {
    /// Matches what shipped before this became configurable.
    fn default() -> Self {
        Self { menu_x: 16, menu_y: 44, panel_x: 400, panel_y: 44, panel_w: 220, default_roll_height: 50 }
    }
}

pub struct LayoutWatcher {
    layout: Layout,
    last_mtime: Option<SystemTime>,
    last_check: Instant,
}

impl LayoutWatcher {
    pub fn new() -> Self {
        let mut w = Self { layout: Layout::default(), last_mtime: None, last_check: Instant::now() };
        w.reload();
        w
    }

    fn reload(&mut self) {
        let text = match std::fs::read_to_string(LAYOUT_PATH) {
            Ok(text) => text,
            Err(_) => return, // missing file -- keep whatever we had (default, on first run)
        };
        match toml::from_str::<Layout>(&text) {
            Ok(layout) => self.layout = layout,
            Err(e) => eprintln!("plaits: layout.toml parse error, keeping previous layout: {e}"),
        }
    }

    /// Cheap to call every frame -- only touches disk a couple of times a
    /// second, and only re-parses when the mtime actually moved.
    pub fn poll(&mut self) {
        if self.last_check.elapsed() < POLL_INTERVAL {
            return;
        }
        self.last_check = Instant::now();
        let mtime = std::fs::metadata(LAYOUT_PATH).and_then(|m| m.modified()).ok();
        if mtime != self.last_mtime {
            self.last_mtime = mtime;
            self.reload();
        }
    }

    pub fn get(&self) -> Layout {
        self.layout
    }
}
