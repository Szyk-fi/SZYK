//! Optional direct MIDI input, selected by the user. I/O and decoding stay on
//! MIDI/UI threads. No callback audio processor ever touches a port or file.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver},
    Arc,
};
#[derive(Clone, Copy, Default)]
struct Channel {
    number: u16,
    value: u16,
    selected: bool,
    rpn_msb: u8,
    rpn_lsb: u8,
    bank: u8,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Parameter(u16, u16),
    Control(u8, u8, u8),
    Note(u8, u8, u8),
    Bend(u8, f32),
    Pressure(u8, u8),
    Program(u8, u8, u8),
    Start,
    Stop,
    Clock,
}
pub struct Decoder {
    channels: [Channel; 16],
}
impl Default for Decoder {
    fn default() -> Self {
        Self {
            channels: [Channel::default(); 16],
        }
    }
}
impl Decoder {
    pub fn message(&mut self, msg: &[u8]) -> Option<Event> {
        let &status = msg.first()?;
        let ch = status & 15;
        let c = &mut self.channels[ch as usize];
        let val = msg.get(2).copied().unwrap_or(0);
        match status {
            0xf8 => Some(Event::Clock),
            0xfa | 0xfb => Some(Event::Start),
            0xfc => Some(Event::Stop),
            0x80..=0x9f if msg.len() == 3 && msg[1] < 128 && val < 128 => Some(Event::Note(
                ch,
                msg[1],
                if status & 0xf0 == 0x90 { val } else { 0 },
            )),
            0xb0..=0xbf if msg.len() == 3 && msg[1] < 128 && val < 128 => match msg[1] {
                99 => {
                    c.number = (c.number & 127) | ((val as u16) << 7);
                    c.selected = true;
                    None
                }
                98 => {
                    c.number = (c.number & !127) | val as u16;
                    c.selected = true;
                    None
                }
                6 if c.selected => {
                    c.value = (c.value & 127) | ((val as u16) << 7);
                    Some(Event::Parameter(c.number, c.value))
                }
                38 if c.selected => {
                    c.value = (c.value & !127) | val as u16;
                    Some(Event::Parameter(c.number, c.value))
                }
                96 if c.selected => {
                    c.value = c.value.saturating_add(1).min(16383);
                    Some(Event::Parameter(c.number, c.value))
                }
                97 if c.selected => {
                    c.value = c.value.saturating_sub(1);
                    Some(Event::Parameter(c.number, c.value))
                }
                101 => {
                    c.rpn_msb = val;
                    if c.rpn_msb == 127 && c.rpn_lsb == 127 {
                        c.selected = false;
                    }
                    None
                }
                100 => {
                    c.rpn_lsb = val;
                    if c.rpn_msb == 127 && c.rpn_lsb == 127 {
                        c.selected = false;
                    }
                    None
                }
                32 => {
                    c.bank = val;
                    None
                }
                _ => Some(Event::Control(ch, msg[1], val)),
            },
            0xc0..=0xcf if msg.len() == 2 && msg[1] < 128 => {
                Some(Event::Program(ch, c.bank, msg[1]))
            }
            0xd0..=0xdf if msg.len() == 2 && msg[1] < 128 => Some(Event::Pressure(ch, msg[1])),
            0xe0..=0xef if msg.len() == 3 && msg[1] < 128 && val < 128 => Some(Event::Bend(
                ch,
                ((msg[1] as i32 + ((val as i32) << 7)) - 8192) as f32 / 8192.,
            )),
            _ => None,
        }
    }
}
pub fn cc_offset(cc: u8) -> Option<usize> {
    Some(match cc {
        3 => 115,
        5 => 18,
        8 => 15,
        9 => 21,
        10 => 209,
        12 => 118,
        13 => 119,
        14 => 130,
        15 => 131,
        16 => 116,
        17 => 117,
        18 => 231,
        19 => 139,
        20 => 0,
        21 => 2,
        22 => 4,
        23 => 8,
        24 => 1,
        25 => 3,
        26 => 5,
        27 => 9,
        28 => 14,
        29 => 16,
        30 => 6,
        31 => 7,
        33 => 136,
        34 => 132,
        35 => 133,
        36 => 134,
        37 => 28,
        39 => 232,
        65 => 19,
        75 => 48,
        76 => 51,
        77 => 49,
        78 => 52,
        85 => 30,
        86 => 34,
        87 => 37,
        88 => 40,
        89 => 43,
        90 => 46,
        102 => 22,
        103 => 23,
        104 => 24,
        105 => 25,
        106 => 32,
        107 => 35,
        108 => 38,
        109 => 41,
        110 => 44,
        111 => 47,
        112 => 50,
        113 => 27,
        114 => 29,
        115 => 33,
        116 => 36,
        117 => 39,
        118 => 42,
        119 => 45,
        _ => return None,
    })
}
pub struct Port {
    pub names: Vec<String>,
    pub selected: usize,
    connection: Option<midir::MidiInputConnection<()>>,
    rx: Receiver<Vec<u8>>,
    overflow: Arc<AtomicBool>,
}
impl Port {
    pub fn new() -> Self {
        let (_, rx) = sync_channel(1);
        Self {
            names: Vec::new(),
            selected: 0,
            connection: None,
            rx,
            overflow: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn scan(&mut self) -> Result<(), String> {
        let input = midir::MidiInput::new("portamax-rev2-probe").map_err(|e| e.to_string())?;
        self.names = input
            .ports()
            .iter()
            .filter_map(|p| input.port_name(p).ok())
            .collect();
        Ok(())
    }
    pub fn select(&mut self, selected: usize) -> Result<(), String> {
        self.connection = None;
        self.selected = 0;
        if selected == 0 {
            return Ok(());
        }
        let mut input = midir::MidiInput::new("portamax-rev2").map_err(|e| e.to_string())?;
        input.ignore(midir::Ignore::None);
        let ports = input.ports();
        let port = ports
            .get(selected - 1)
            .ok_or("MIDI port disappeared; rescan")?;
        let (tx, rx) = sync_channel(256);
        let over = self.overflow.clone();
        self.connection = Some(
            input
                .connect(
                    port,
                    "rev2-input",
                    move |_, msg, _| {
                        if tx.try_send(msg.to_vec()).is_err() {
                            over.store(true, Ordering::Relaxed);
                        }
                    },
                    (),
                )
                .map_err(|e| e.to_string())?,
        );
        self.rx = rx;
        self.selected = selected;
        Ok(())
    }
    pub fn pop(&self) -> Option<Vec<u8>> {
        self.rx.try_recv().ok()
    }
    pub fn overflowed(&self) -> bool {
        self.overflow.swap(false, Ordering::Relaxed)
    }
    pub fn name(&self) -> String {
        if self.selected == 0 {
            "Off (device note bus)".into()
        } else {
            self.names
                .get(self.selected - 1)
                .cloned()
                .unwrap_or_else(|| "Disconnected".into())
        }
    }
}
