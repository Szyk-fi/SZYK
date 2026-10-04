//! Cards: what's in the I/O slots. One row per slot with the card that
//! was read from its EEPROM at boot (or why it wasn't), then every input
//! and output that card added, with the outputs' live values -- so you can
//! see a Portal cable or a Tides output actually move a CV jack.
//!
//! In the sim, left/right on a slot row picks a different card for that
//! slot, the way you'd swap one on the device. Like the device, cards are
//! only read at power-up (there's no hot-plug), so the change applies at
//! the next start; it's saved to `saves/io_slots.json`.

use crate::app::{App, Input};
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::io_cards::{catalog, tier, CardDef, IoCards, Kind, SlotConfig, SlotState, SLOTS};
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use crate::util::AtomicF32;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::path::PathBuf;
use std::sync::Arc;

const BG: Rgb565 = Rgb565::new(3, 5, 4);
const INK: Rgb565 = Rgb565::new(26, 54, 26);
const ACCENT: Rgb565 = Rgb565::new(30, 40, 8);
const DIM: Rgb565 = Rgb565::new(12, 26, 14);

enum Row {
    Slot(usize),
    Port(usize),
}

pub struct CardsApp {
    cards: Arc<IoCards>,
    catalog: Vec<CardDef>,
    /// The slots as they'll be at the next start (sim only).
    pending: SlotConfig,
    /// The slots as they were read at boot.
    booted: SlotConfig,
    config_path: Option<PathBuf>,
    list: ParamList,
    nav_speed: Arc<AtomicF32>,
}

impl CardsApp {
    pub fn new(cards: Arc<IoCards>, nav_speed: Arc<AtomicF32>, config_path: Option<PathBuf>) -> CardsApp {
        let pending = config_path.as_deref().map(SlotConfig::load).unwrap_or_default();
        CardsApp { cards, catalog: catalog(), booted: pending.clone(), pending, config_path, list: ParamList::new(), nav_speed }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for s in 0..SLOTS.len() {
            rows.push(Row::Slot(s));
            for (i, p) in self.cards.ports.iter().enumerate() {
                if p.slot == s {
                    rows.push(Row::Port(i));
                }
            }
        }
        rows
    }

    fn booted_name(&self, s: usize) -> String {
        match self.cards.states.get(s) {
            Some(SlotState::Card(d)) => d.name.clone(),
            Some(SlotState::Fault(e)) => format!("fault: {e}"),
            _ => "empty".into(),
        }
    }

    fn pending_name(&self, s: usize) -> String {
        let id = self.pending.card(s);
        if id.is_empty() {
            return "empty".into();
        }
        self.catalog.iter().find(|d| d.id == id).map_or_else(|| id.to_string(), |d| d.name.clone())
    }

    /// Whether slot `s` will hold something else after a restart.
    fn changed(&self, s: usize) -> bool {
        self.booted.card(s) != self.pending.card(s)
    }

    fn port_value(&self, i: usize) -> String {
        let p = &self.cards.ports[i];
        match (&p.level, p.kind) {
            (Some(l), Kind::GateOut) => if l.get() > 0.5 { "high".into() } else { "low".into() },
            // A full 0..1 of modulation is the jack's whole +5 V; the
            // outputs swing +-5 V.
            (Some(l), _) => format!("{:+.2} V", (l.get() * 10.0).clamp(-5.0, 5.0)),
            (None, Kind::AudioIn | Kind::CvIn | Kind::GateIn) => "source".into(),
            (None, Kind::MidiOut) => "instrument".into(),
            (None, Kind::MidiIn) => "note source".into(),
            (None, _) => "main out".into(),
        }
    }

    fn display(&self) -> Vec<(String, String)> {
        self.rows()
            .iter()
            .map(|r| match r {
                Row::Slot(s) => {
                    let mut v = self.booted_name(*s);
                    if self.changed(*s) {
                        v = format!("{v} -> {} at restart", self.pending_name(*s));
                    }
                    (format!("Slot {} ({})", SLOTS[*s], tier(*s)), v)
                }
                Row::Port(i) => {
                    let name = &self.cards.ports[*i].name;
                    // "Slot B: CV Out 1" / "Slot B CV Out 1" -> "CV Out 1"
                    let short = name.splitn(3, ' ').nth(2).unwrap_or(name).trim_start_matches(':').trim();
                    (format!("  {short}"), self.port_value(*i))
                }
            })
            .collect()
    }

    /// Steps slot `s`'s card through the catalog and "empty".
    fn swap(&mut self, s: usize, d: i32) {
        let mut ids: Vec<String> = vec![String::new()];
        ids.extend(self.catalog.iter().map(|c| c.id.clone()));
        let cur = ids.iter().position(|i| i == self.pending.card(s)).unwrap_or(0) as i32;
        let next = ids[(cur + d.signum()).rem_euclid(ids.len() as i32) as usize].clone();
        while self.pending.slots.len() <= s {
            self.pending.slots.push(String::new());
        }
        self.pending.slots[s] = next;
        if let Some(path) = &self.config_path {
            self.pending.save(path);
        }
    }
}

impl App for CardsApp {
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display().into_iter().map(|(a, b)| (a, b, false)).collect()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.slint_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        (rows[start..end].to_vec(), self.list.selected - start, start > 0, end < rows.len())
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let mut cells = Vec::new();
        for s in 0..SLOTS.len() {
            cells.push(format!("Slot {}", SLOTS[s]));
            cells.push(self.booted_name(s));
            let ports = self.cards.ports.iter().filter(|p| p.slot == s).count();
            cells.push(match self.cards.states.get(s) {
                Some(SlotState::Card(d)) => format!("{ports} ports · {} mA", d.current_ma),
                Some(SlotState::Fault(_)) => "off".into(),
                _ => format!("{} tier", tier(s)),
            });
        }
        let pending = (0..SLOTS.len()).any(|s| self.changed(s));
        let selected_slot = match self.rows().get(self.list.selected) {
            Some(Row::Slot(s)) => Some(*s),
            Some(Row::Port(i)) => Some(self.cards.ports[*i].slot),
            None => None,
        };
        crate::app::SlintExtra::Grid(crate::app::GridExtra {
            caption: format!("I/O CARDS / +5 V {} OF {} mA", self.cards.current_ma(), crate::io_cards::FIVE_V_BUDGET_MA),
            title: selected_slot
                .and_then(|s| match &self.cards.states[s] {
                    SlotState::Card(d) => Some(format!("{} · rev {} · #{:04X}", d.name, d.revision, d.serial)),
                    _ => None,
                })
                .unwrap_or_else(|| "No card".into()),
            cells,
            col_x: vec![0.0, 60.0, 170.0],
            highlight: selected_slot.map_or(-1, |s| s as i32),
            footer: if pending { "Card changes apply at the next start".into() } else { "Left/right on a slot row swaps its card (sim)".into() },
            meter: -1.0,
        })
    }

    fn tick(&mut self, input: &Input) {
        let rows = self.rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        if let Some(Row::Slot(s)) = rows.get(self.list.selected) {
            let s = *s;
            if input.knob2 != 0 {
                self.swap(s, input.knob2);
            }
            if input.knob2_press {
                // Back to what's actually in the slot.
                let booted = self.booted.card(s).to_string();
                while self.pending.slots.len() <= s {
                    self.pending.slots.push(String::new());
                }
                self.pending.slots[s] = booted;
                if let Some(path) = &self.config_path {
                    self.pending.save(path);
                }
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(BG)).draw(fb).ok();
        Text::new("Cards", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(fb).ok();
        let rows = self.display();
        self.list.draw_themed(fb, 16, 44, 24, 11, &rows, BG, DIM, ACCENT);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new(&format!("+5 V load {} mA", self.cards.current_ma()), Point::new(420, 30), small).draw(fb).ok();
        Text::new("U/D: browse   L/R on a slot: swap card (applies at restart)   hold SELECT: undo", Point::new(16, 337), small).draw(fb).ok();
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    // The device and the sim boot the cards before any app (main.rs).
    let cards = ctx.try_get::<IoCards>().unwrap_or_else(|| {
        if cfg!(test) {
            return Arc::new(IoCards::empty());
        }
        // Show what's in the slots without registering anything: the
        // registration belongs to startup.
        Arc::new(IoCards::install(IoCards::detect(IoCards::sim_reader(&SlotConfig::load(&SlotConfig::path()), &catalog())), &crate::audio_bus::AudioBus::new(), &crate::modbus::ModBus::new(), None))
    });
    Box::new(CardsApp::new(cards, ctx.named("nav_speed"), (!cfg!(test)).then(SlotConfig::path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_bus::AudioBus;
    use crate::modbus::ModBus;

    fn booted() -> (CardsApp, Arc<ModBus>) {
        let audio = AudioBus::new();
        let mods = Arc::new(ModBus::new());
        let states = IoCards::detect(IoCards::sim_reader(&SlotConfig::default(), &catalog()));
        let cards = Arc::new(IoCards::install(states, &audio, &mods, None));
        (CardsApp::new(cards, Arc::new(AtomicF32::new(1.0)), None), mods)
    }

    #[test]
    fn it_lists_every_slot_and_shows_a_patched_cv_out_moving() {
        let (mut app, mods) = booted();
        let rows = app.display();
        assert_eq!(rows[0], ("Slot A (upper)".to_string(), "Audio 2x2".to_string()));
        assert!(rows.iter().any(|r| r.0 == "Slot B (upper)" && r.1 == "CV 4x4"));
        assert!(rows.iter().any(|r| r.0 == "Slot C (upper)" && r.1 == "MIDI DIN"));
        assert!(rows.iter().any(|r| r.0 == "Slot F (lower)" && r.1 == "empty"));
        mods.get(mods.index_of("Slot B: CV Out 2").unwrap()).unwrap().set(0.25);
        let rows = app.display();
        assert!(rows.iter().any(|r| r.0 == "  CV Out 2" && r.1 == "+2.50 V"), "{rows:?}");
        app.tick(&Input::default());
    }

    #[test]
    fn swapping_a_card_waits_for_a_restart() {
        let (mut app, _) = booted();
        app.list.selected = 0; // Slot A
        app.tick(&Input { knob2: 1, ..Default::default() });
        let rows = app.display();
        assert!(rows[0].1.starts_with("Audio 2x2 -> ") && rows[0].1.ends_with("at restart"), "{:?}", rows[0]);
        app.tick(&Input { knob2_press: true, ..Default::default() });
        assert_eq!(app.display()[0].1, "Audio 2x2", "hold SELECT puts it back");
    }
}
