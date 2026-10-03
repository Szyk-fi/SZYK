//! Swappable I/O cards: the jacks live on small cards that plug into the
//! main board, so I/O can be changed or repaired without touching the
//! rest of the unit. See docs/IO_CARDS.md for the connector and the
//! electrical side.
//!
//! Every card carries a small I2C EEPROM describing itself. At boot the
//! firmware reads each slot's EEPROM, checks it, switches on that slot's
//! +5 V only if it's a valid Portamax card, and registers what the card
//! offers with the rest of the system:
//! - audio inputs, CV inputs and gate inputs become sources on the audio
//!   bus (so Portal and every effect's Source row can use them);
//! - CV and gate outputs become modulation inputs ("Slot B: CV Out 1"),
//!   so any app's outputs, Portal cables, Marbles or Tides can drive them;
//! - MIDI outputs become instruments on the note bus, MIDI inputs note
//!   sources.
//! Apps never know about cards; they just see more sources and targets.
//!
//! This module holds the EEPROM format (`CardImage::encode`/`decode`,
//! shared with the firmware), the card definitions (`assets/io_cards/`,
//! plus third-party ones on the SD card in `saves/io_cards/`), and the
//! sim's slots, set in `saves/io_slots.json`. In the sim a slot's
//! "EEPROM" is the image encoded from its card definition, read back
//! through the same decoder the firmware will use. Card inputs read
//! silence in the sim, except that the first audio input carries the
//! computer's audio input.

#![allow(dead_code)] // parts are used only by the Slint GUI or the firmware path

use crate::audio_bus::AudioBus;
use crate::modbus::ModBus;
use crate::note_bus::{NoteBus, NoteInboxRef, NoteOut};
use crate::util::AtomicF32;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The slots on the main board, left to right.
pub const SLOTS: [char; 3] = ['A', 'B', 'C'];

/// EEPROM image layout, version 1 (little-endian):
///
/// | Offset | Size | Field |
/// |---|---|---|
/// | 0 | 4 | magic `PMXC` |
/// | 4 | 1 | format version (1) |
/// | 5 | 1 | reserved, 0 |
/// | 6 | 2 | vendor id |
/// | 8 | 2 | product id |
/// | 10 | 1 | hardware revision |
/// | 11 | 1 | +5 V current, in 10 mA units |
/// | 12 | 4 | serial number |
/// | 16 | 24 | name, UTF-8, zero-padded |
/// | 40 | 1 | resource count n |
/// | 41 | 1 | reserved, 0 |
/// | 42 | 6n | resources: kind, count, lane, flags, param (u16) |
/// | 42+6n | 2 | CRC-16/CCITT-FALSE of everything before it |
///
/// A 24C02 (256 bytes) holds up to 35 resource entries.
pub const MAGIC: &[u8; 4] = b"PMXC";
pub const FORMAT_VERSION: u8 = 1;
const HEADER: usize = 42;
const NAME_LEN: usize = 24;
pub const EEPROM_SIZE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    AudioIn = 1,
    AudioOut = 2,
    CvIn = 3,
    CvOut = 4,
    GateIn = 5,
    GateOut = 6,
    MidiIn = 7,
    MidiOut = 8,
}

impl Kind {
    fn from_u8(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::AudioIn,
            2 => Kind::AudioOut,
            3 => Kind::CvIn,
            4 => Kind::CvOut,
            5 => Kind::GateIn,
            6 => Kind::GateOut,
            7 => Kind::MidiIn,
            8 => Kind::MidiOut,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::AudioIn => "Audio In",
            Kind::AudioOut => "Audio Out",
            Kind::CvIn => "CV In",
            Kind::CvOut => "CV Out",
            Kind::GateIn => "Gate In",
            Kind::GateOut => "Gate Out",
            Kind::MidiIn => "MIDI In",
            Kind::MidiOut => "MIDI Out",
        }
    }
}

/// Which group of connector pins a resource uses (docs/IO_CARDS.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Sai = 0,
    Spi = 1,
    Uart = 2,
    Gpio = 3,
    I2c = 4,
}

impl Lane {
    fn from_u8(v: u8) -> Option<Lane> {
        Some(match v {
            0 => Lane::Sai,
            1 => Lane::Spi,
            2 => Lane::Uart,
            3 => Lane::Gpio,
            4 => Lane::I2c,
            _ => return None,
        })
    }
}

/// One kind of I/O on a card: `count` channels of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub kind: Kind,
    pub count: u8,
    pub lane: Lane,
    /// Swings both ways around 0 V (CV).
    #[serde(default)]
    pub bipolar: bool,
    /// Audio: sample rate in Hz. CV: full-scale span in millivolts
    /// (10000 = +-5 V when bipolar, 0..10 V when not). Stored as
    /// rate / 100 or mV / 1 in the image's u16.
    #[serde(default)]
    pub param: u32,
}

/// A card definition: what gets written into its EEPROM at the factory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardDef {
    /// File id, e.g. "audio_2x2" (not stored in the EEPROM).
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    #[serde(default = "one")]
    pub revision: u8,
    /// What the card draws from the slot's +5 V, in mA.
    pub current_ma: u16,
    #[serde(default)]
    pub serial: u32,
    #[serde(default)]
    pub description: String,
    pub resources: Vec<Resource>,
}

fn one() -> u8 {
    1
}

#[derive(Debug, PartialEq)]
pub enum ImageError {
    Blank,
    BadMagic,
    UnsupportedVersion(u8),
    Truncated,
    BadCrc,
    BadResource(usize),
    TooBig,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Blank => write!(f, "blank EEPROM"),
            ImageError::BadMagic => write!(f, "not a Portamax card"),
            ImageError::UnsupportedVersion(v) => write!(f, "card format v{v} is newer than this firmware"),
            ImageError::Truncated => write!(f, "EEPROM data cut short"),
            ImageError::BadCrc => write!(f, "EEPROM checksum failed"),
            ImageError::BadResource(i) => write!(f, "unknown I/O entry {}", i + 1),
            ImageError::TooBig => write!(f, "too many I/O entries for the EEPROM"),
        }
    }
}

/// CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF. Cheap on any MCU.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

pub struct CardImage;

impl CardImage {
    pub fn encode(def: &CardDef) -> Result<Vec<u8>, ImageError> {
        let n = def.resources.len();
        let len = HEADER + 6 * n + 2;
        if len > EEPROM_SIZE || n > 255 {
            return Err(ImageError::TooBig);
        }
        let mut b = Vec::with_capacity(len);
        b.extend_from_slice(MAGIC);
        b.push(FORMAT_VERSION);
        b.push(0);
        b.extend_from_slice(&def.vendor.to_le_bytes());
        b.extend_from_slice(&def.product.to_le_bytes());
        b.push(def.revision);
        b.push((def.current_ma / 10).min(255) as u8);
        b.extend_from_slice(&def.serial.to_le_bytes());
        let mut name = [0u8; NAME_LEN];
        // Truncate on a character boundary so the name stays valid UTF-8.
        let mut end = def.name.len().min(NAME_LEN);
        while !def.name.is_char_boundary(end) {
            end -= 1;
        }
        name[..end].copy_from_slice(&def.name.as_bytes()[..end]);
        b.extend_from_slice(&name);
        b.push(n as u8);
        b.push(0);
        for r in &def.resources {
            let param = match r.kind {
                Kind::AudioIn | Kind::AudioOut => r.param / 100,
                _ => r.param,
            }
            .min(u16::MAX as u32) as u16;
            b.push(r.kind as u8);
            b.push(r.count);
            b.push(r.lane as u8);
            b.push(r.bipolar as u8);
            b.extend_from_slice(&param.to_le_bytes());
        }
        let crc = crc16(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        Ok(b)
    }

    /// Reads an image (any trailing bytes, e.g. the rest of a 256-byte
    /// EEPROM, are ignored).
    pub fn decode(b: &[u8]) -> Result<CardDef, ImageError> {
        if b.len() >= 4 && b[..4].iter().all(|&x| x == 0xFF) {
            return Err(ImageError::Blank);
        }
        if b.len() < HEADER + 2 {
            return Err(ImageError::Truncated);
        }
        if &b[..4] != MAGIC {
            return Err(ImageError::BadMagic);
        }
        if b[4] != FORMAT_VERSION {
            return Err(ImageError::UnsupportedVersion(b[4]));
        }
        let n = b[40] as usize;
        let end = HEADER + 6 * n;
        if b.len() < end + 2 {
            return Err(ImageError::Truncated);
        }
        let stored = u16::from_le_bytes([b[end], b[end + 1]]);
        if crc16(&b[..end]) != stored {
            return Err(ImageError::BadCrc);
        }
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let name_bytes = &b[16..16 + NAME_LEN];
        let name_len = name_bytes.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
        let mut resources = Vec::with_capacity(n);
        for i in 0..n {
            let o = HEADER + 6 * i;
            let kind = Kind::from_u8(b[o]).ok_or(ImageError::BadResource(i))?;
            let lane = Lane::from_u8(b[o + 2]).ok_or(ImageError::BadResource(i))?;
            let raw = u16_at(o + 4) as u32;
            let param = match kind {
                Kind::AudioIn | Kind::AudioOut => raw * 100,
                _ => raw,
            };
            resources.push(Resource { kind, count: b[o + 1], lane, bipolar: b[o + 3] & 1 != 0, param });
        }
        Ok(CardDef {
            id: String::new(),
            name: String::from_utf8_lossy(&name_bytes[..name_len]).into_owned(),
            vendor: u16_at(6),
            product: u16_at(8),
            revision: b[10],
            current_ma: b[11] as u16 * 10,
            serial: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            description: String::new(),
            resources,
        })
    }
}

/// The bundled card definitions, plus any on the SD card.
pub fn catalog() -> Vec<CardDef> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [root.join("assets/io_cards"), root.join("saves/io_cards")] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for f in files {
            match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<CardDef>(&t).map_err(|e| e.to_string())) {
                Ok(mut def) => {
                    if def.id.is_empty() {
                        def.id = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    }
                    out.retain(|d: &CardDef| d.id != def.id);
                    out.push(def);
                }
                Err(e) => eprintln!("io cards: {}: {e}", f.display()),
            }
        }
    }
    out
}

/// Which card is in which slot in the sim (`saves/io_slots.json`). On the
/// device this is whatever is physically plugged in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotConfig {
    /// One card id (or "" for empty) per slot, A first.
    pub slots: Vec<String>,
}

impl Default for SlotConfig {
    /// The standard build: audio, CV and MIDI.
    fn default() -> Self {
        SlotConfig { slots: vec!["audio_2x2".into(), "cv_4x4".into(), "midi_din".into()] }
    }
}

impl SlotConfig {
    pub fn path() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/io_slots.json"))
    }
    pub fn load(path: &Path) -> SlotConfig {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, t);
        }
    }
    pub fn card(&self, slot: usize) -> &str {
        self.slots.get(slot).map(String::as_str).unwrap_or("")
    }
}

/// What a slot turned out to hold at boot.
#[derive(Clone, Debug)]
pub enum SlotState {
    Empty,
    Card(CardDef),
    /// Something is plugged in but its EEPROM didn't check out: the slot
    /// stays unpowered.
    Fault(String),
}

/// One registered piece of I/O, for the Cards screen.
pub struct Port {
    pub slot: usize,
    pub kind: Kind,
    pub name: String,
    /// CV/gate outputs: the value apps write (0..1).
    pub level: Option<Arc<AtomicF32>>,
}

/// The detected cards and everything they registered. Kept alive for the
/// whole run so the registrations stay valid.
pub struct IoCards {
    pub states: Vec<SlotState>,
    pub ports: Vec<Port>,
    /// The audio-bus buffer the computer's audio input fills, in the sim.
    pub host_input: Option<Arc<Mutex<Vec<f32>>>>,
    /// Kept so the note-bus registrations stay alive.
    midi_out: Vec<Arc<NoteInboxRef>>,
    midi_in: Vec<NoteOut>,
    audio_in: Vec<Arc<Mutex<Vec<f32>>>>,
}

impl IoCards {
    pub fn empty() -> IoCards {
        IoCards { states: SLOTS.iter().map(|_| SlotState::Empty).collect(), ports: Vec::new(), host_input: None, midi_out: Vec::new(), midi_in: Vec::new(), audio_in: Vec::new() }
    }

    /// Reads every slot. `read_eeprom(slot)` returns the slot's EEPROM
    /// contents, or None when nothing is plugged in (PRESENT# high).
    pub fn detect(read_eeprom: impl Fn(usize) -> Option<Vec<u8>>) -> Vec<SlotState> {
        (0..SLOTS.len())
            .map(|s| match read_eeprom(s) {
                None => SlotState::Empty,
                Some(bytes) => match CardImage::decode(&bytes) {
                    Ok(def) => SlotState::Card(def),
                    Err(e) => SlotState::Fault(e.to_string()),
                },
            })
            .collect()
    }

    /// The sim's EEPROM reader: each configured slot's card definition,
    /// encoded as it would be at the factory.
    pub fn sim_reader(config: &SlotConfig, catalog: &[CardDef]) -> impl Fn(usize) -> Option<Vec<u8>> {
        let images: Vec<Option<Vec<u8>>> = (0..SLOTS.len())
            .map(|s| {
                let id = config.card(s);
                if id.is_empty() {
                    return None;
                }
                Some(match catalog.iter().find(|d| d.id == id) {
                    Some(def) => {
                        let mut def = def.clone();
                        def.serial = 0x5000 + s as u32;
                        CardImage::encode(&def).unwrap_or_default()
                    }
                    // A card the sim doesn't know reads as a blank EEPROM.
                    None => vec![0xFF; 8],
                })
            })
            .collect();
        move |s| images.get(s).cloned().flatten()
    }

    /// Registers every detected card's I/O on the buses.
    pub fn install(states: Vec<SlotState>, audio: &AudioBus, mods: &ModBus, notes: Option<&NoteBus>) -> IoCards {
        let mut cards = IoCards { states, ..IoCards::empty() };
        let states = cards.states.clone();
        for (s, state) in states.iter().enumerate() {
            let SlotState::Card(def) = state else { continue };
            let slot = SLOTS[s];
            for r in &def.resources {
                for ch in 1..=r.count as usize {
                    let numbered = |label: &str| if r.count > 1 { format!("{label} {ch}") } else { label.to_string() };
                    match r.kind {
                        Kind::AudioIn => {
                            // Audio buses are mono: one source per stereo pair.
                            if ch % 2 == 0 {
                                continue;
                            }
                            let name = if r.count > 1 { format!("Slot {slot} Audio In {}-{}", ch, ch + 1) } else { format!("Slot {slot} Audio In") };
                            let buf = audio.register(name.clone());
                            if cards.host_input.is_none() {
                                cards.host_input = Some(Arc::clone(&buf));
                            }
                            cards.audio_in.push(buf);
                            cards.ports.push(Port { slot: s, kind: r.kind, name, level: None });
                        }
                        Kind::CvIn | Kind::GateIn => {
                            let name = format!("Slot {slot} {}", numbered(r.kind.label()));
                            cards.audio_in.push(audio.register(name.clone()));
                            cards.ports.push(Port { slot: s, kind: r.kind, name, level: None });
                        }
                        Kind::CvOut | Kind::GateOut => {
                            let name = format!("Slot {slot}: {}", numbered(r.kind.label()));
                            let level = mods.register(name.clone());
                            cards.ports.push(Port { slot: s, kind: r.kind, name, level: Some(level) });
                        }
                        Kind::MidiOut => {
                            let name = format!("Slot {slot} {}", numbered("MIDI Out"));
                            if let Some(bus) = notes {
                                if let Some(inbox) = bus.register_instrument(&format!("slot_{}", slot.to_ascii_lowercase()), &name) {
                                    cards.midi_out.push(inbox);
                                }
                            }
                            cards.ports.push(Port { slot: s, kind: r.kind, name, level: None });
                        }
                        Kind::MidiIn => {
                            let name = format!("Slot {slot} {}", numbered("MIDI In"));
                            if let Some(bus) = notes {
                                cards.midi_in.push(bus.register_source(&name));
                            }
                            cards.ports.push(Port { slot: s, kind: r.kind, name, level: None });
                        }
                        Kind::AudioOut => {
                            // The main outputs: every app's audio already
                            // goes there, so nothing to register.
                            if ch % 2 == 1 {
                                cards.ports.push(Port { slot: s, kind: r.kind, name: format!("Slot {slot} {}", if r.count > 1 { format!("Audio Out {}-{}", ch, ch + 1) } else { "Audio Out".into() }), level: None });
                            }
                        }
                    }
                }
            }
        }
        cards
    }

    /// The whole sim boot: read the slot config, detect, install.
    pub fn boot(audio: &AudioBus, mods: &ModBus, notes: Option<&NoteBus>) -> IoCards {
        let config = SlotConfig::load(&SlotConfig::path());
        let states = IoCards::detect(IoCards::sim_reader(&config, &catalog()));
        IoCards::install(states, audio, mods, notes)
    }

    /// The +5 V the cards draw in total, for the power budget.
    pub fn current_ma(&self) -> u32 {
        self.states.iter().map(|s| if let SlotState::Card(d) = s { d.current_ma as u32 } else { 0 }).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str) -> CardDef {
        catalog().into_iter().find(|d| d.id == id).unwrap_or_else(|| panic!("{id} in assets/io_cards"))
    }

    #[test]
    fn crc_matches_the_standard_check_value() {
        // CRC-16/CCITT-FALSE of "123456789" is 0x29B1.
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn every_bundled_card_round_trips_through_its_eeprom_image() {
        let cards = catalog();
        assert!(cards.len() >= 3, "{cards:?}");
        for c in cards {
            let img = CardImage::encode(&c).unwrap();
            assert!(img.len() <= EEPROM_SIZE, "{} fits a 24C02", c.id);
            let mut padded = img.clone();
            padded.resize(EEPROM_SIZE, 0xFF);
            let back = CardImage::decode(&padded).unwrap();
            assert_eq!(back.name, c.name);
            assert_eq!(back.resources, c.resources, "{}", c.id);
            assert_eq!(back.current_ma, c.current_ma / 10 * 10);
        }
    }

    #[test]
    fn a_damaged_or_foreign_eeprom_is_refused() {
        let img = CardImage::encode(&def("cv_4x4")).unwrap();
        let mut flipped = img.clone();
        flipped[20] ^= 0x40;
        assert_eq!(CardImage::decode(&flipped), Err(ImageError::BadCrc));
        assert_eq!(CardImage::decode(&[0xFF; 256]), Err(ImageError::Blank));
        let mut foreign = img.clone();
        foreign[0] = b'X';
        assert_eq!(CardImage::decode(&foreign), Err(ImageError::BadMagic));
        let mut newer = img;
        newer[4] = 9;
        assert_eq!(CardImage::decode(&newer), Err(ImageError::UnsupportedVersion(9)));
    }

    #[test]
    fn detected_cards_show_up_on_the_buses_apps_already_use() {
        let audio = AudioBus::new();
        let mods = ModBus::new();
        let notes = NoteBus::new();
        let config = SlotConfig::default();
        let states = IoCards::detect(IoCards::sim_reader(&config, &catalog()));
        let cards = IoCards::install(states, &audio, &mods, Some(&notes));
        assert!(cards.states.iter().all(|s| matches!(s, SlotState::Card(_))), "all three slots read");
        // Inputs are sources Portal and effects can pick.
        let names = audio.names();
        assert!(names.contains(&"Slot A Audio In 1-2".to_string()), "{names:?}");
        assert!(names.contains(&"Slot B CV In 1".to_string()), "{names:?}");
        assert!(names.contains(&"Slot B Gate In 1".to_string()));
        // Outputs are mod inputs any app can patch to.
        assert!(mods.index_of("Slot B: CV Out 4").is_some());
        // MIDI out is an instrument; MIDI in a note source.
        assert!(notes.instrument_index("Slot C MIDI Out").is_some());
        assert!(notes.sources().iter().any(|(n, _, _)| n == "Slot C MIDI In"));
        assert!(cards.host_input.is_some());
        // A patched value reaches the card's output.
        let i = mods.index_of("Slot B: CV Out 1").unwrap();
        mods.get(i).unwrap().set(0.7);
        let port = cards.ports.iter().find(|p| p.name == "Slot B: CV Out 1").unwrap();
        assert_eq!(port.level.as_ref().unwrap().get(), 0.7);
    }

    #[test]
    fn empty_and_unknown_slots_register_nothing() {
        let audio = AudioBus::new();
        let mods = ModBus::new();
        let config = SlotConfig { slots: vec!["".into(), "no_such_card".into()] };
        let states = IoCards::detect(IoCards::sim_reader(&config, &catalog()));
        assert!(matches!(states[0], SlotState::Empty));
        assert!(matches!(&states[1], SlotState::Fault(e) if e == "blank EEPROM"));
        assert!(matches!(states[2], SlotState::Empty));
        let cards = IoCards::install(states, &audio, &mods, None);
        assert!(cards.ports.is_empty());
        assert_eq!(audio.len(), 0);
        assert_eq!(cards.current_ma(), 0);
    }
}
