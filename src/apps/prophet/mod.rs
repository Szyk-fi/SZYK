//! Rev2-compatible software instrument. Native two-layer SysEx patches are
//! retained losslessly; the audio model is original, not hardware firmware.
mod dsp;
mod midi;
mod panel;
mod patch;
mod presets;
mod spec;
#[cfg(test)]
mod tests;
mod tuning;
use crate::{
    app::{App, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    util::AtomicF32,
};
use patch::Patch;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
const MOD_NAMES: [&str; 8] = [
    "Cutoff",
    "Resonance",
    "Osc 1 Shape Mod",
    "Osc 2 Shape Mod",
    "Osc Mix",
    "Sub",
    "Noise",
    "Filter Env Amount",
];
const SECTIONS: [&str; 16] = [
    "Oscillators",
    "Filter / amplifier",
    "Envelopes",
    "LFOs",
    "Mod matrix",
    "Controllers",
    "Effects",
    "Voice / clock",
    "Arpeggiator",
    "Gated sequence",
    "Poly notes 1-2",
    "Poly notes 3-4",
    "Poly notes 5-6",
    "Poly velocities",
    "Programs",
    "Performance",
];
#[derive(Clone, Copy)]
struct State {
    patch: Patch,
    notes: [u8; 128],
    bend: f32,
    wheel: f32,
    pressure: f32,
    breath: f32,
    foot: f32,
    expression: f32,
    generation: u32,
    playing: bool,
    hold: bool,
    voices: usize,
    follow_clock: bool,
    mono: bool,
    layer: usize,
    tuning: [f32; 128],
    master_coarse: f32,
    master_fine: f32,
    multi: bool,
    midi_notes: [[u8; 128]; 2],
}
impl Default for State {
    fn default() -> Self {
        Self {
            patch: Patch::default(),
            notes: [0; 128],
            bend: 0.,
            wheel: 0.,
            pressure: 0.,
            breath: 0.,
            foot: 0.,
            expression: 0.,
            generation: 0,
            playing: false,
            hold: false,
            voices: 16,
            follow_clock: false,
            mono: false,
            layer: 0,
            tuning: tuning::table(0),
            master_coarse: 0.,
            master_fine: 0.,
            multi: false,
            midi_notes: [[0; 128]; 2],
        }
    }
}
struct Shared {
    state: Mutex<State>,
    mods: Vec<Arc<AtomicF32>>,
    output: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicF32>,
    external: Arc<AtomicF32>,
    peak: AtomicF32,
    tail: AtomicBool,
}
#[derive(Clone)]
struct Program {
    patch: Patch,
    source: String,
}
#[derive(Clone)]
struct Row {
    name: String,
    value: String,
    offset: Option<usize>,
    action: usize,
}
pub struct ProphetApp {
    shared: Arc<Shared>,
    state: State,
    programs: Vec<Program>,
    baseline: Patch,
    compare: Option<Patch>,
    program: usize,
    section: usize,
    selected: usize,
    layer: usize,
    seq_page: usize,
    track: usize,
    slot: usize,
    save_dir: PathBuf,
    import_dir: PathBuf,
    status: String,
    input: Input,
    octave: i32,
    transpose: i32,
    held: [u8; 128],
    pointer: Option<u8>,
    record: bool,
    record_step: usize,
    tuning_index: usize,
    midi: midi::Port,
    decoder: midi::Decoder,
    midi_channel: usize,
    sustain: bool,
    midi_keys: [[u8; 128]; 2],
    midi_bend: f32,
    midi_wheel: f32,
    midi_pressure: f32,
}
impl ProphetApp {
    pub fn new(mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dirs(
            mods,
            bus,
            mixer,
            Path::new("patches/prophet-rev2"),
            Path::new("saves/prophet-rev2"),
        )
    }
    fn with_dirs(
        mods: Arc<ModBus>,
        bus: Arc<AudioBus>,
        mixer: Arc<MixerBus>,
        import: &Path,
        save: &Path,
    ) -> Self {
        let (level, external) = mixer.register("Rev2", &mods);
        let mods = (0..2)
            .flat_map(|l| {
                MOD_NAMES
                    .iter()
                    .map(move |n| format!("Rev2: {} {n}", if l == 0 { "A" } else { "B" }))
            })
            .map(|n| mods.register(n))
            .collect();
        let output = bus.register("Rev2");
        output.lock().unwrap().reserve(8192);
        let state = State::default();
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            mods,
            output,
            level,
            external,
            peak: AtomicF32::new(0.),
            tail: AtomicBool::new(false),
        });
        let mut a = Self {
            shared,
            state,
            programs: presets::factory(),
            baseline: state.patch,
            compare: None,
            program: 0,
            section: 0,
            selected: 0,
            layer: 0,
            seq_page: 0,
            track: 0,
            slot: 0,
            save_dir: save.into(),
            import_dir: import.into(),
            status: String::new(),
            input: Input::default(),
            octave: 0,
            transpose: 0,
            held: [0; 128],
            pointer: None,
            record: false,
            record_step: 0,
            tuning_index: 0,
            midi: midi::Port::new(),
            decoder: midi::Decoder::default(),
            midi_channel: 0,
            sustain: false,
            midi_keys: [[0; 128]; 2],
            midi_bend: 0.,
            midi_wheel: 0.,
            midi_pressure: 0.,
        };
        a.scan();
        a.load(0);
        a
    }
    fn publish(&self) {
        *self.shared.state.lock().unwrap() = self.state;
    }
    fn scan_path(dir: &Path, programs: &mut Vec<Program>, errors: &mut Vec<String>, depth: usize) {
        if depth > 8 {
            errors.push(format!("Directory nesting exceeds 8: {}", dir.display()));
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                Self::scan_path(&path, programs, errors, depth + 1);
            } else if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("syx"))
            {
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| patch::import(&b))
                {
                    Ok(patches) => {
                        for patch in patches {
                            programs.push(Program {
                                patch,
                                source: path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into(),
                            });
                        }
                    }
                    Err(e) => errors.push(format!("{}: {e}", path.display())),
                }
            }
        }
    }
    fn scan(&mut self) {
        let mut p = presets::factory();
        let mut errors = Vec::new();
        Self::scan_path(&self.import_dir, &mut p, &mut errors, 0);
        Self::scan_path(&self.save_dir, &mut p, &mut errors, 0);
        self.programs = p;
        self.program = self.program.min(self.programs.len() - 1);
        self.status = if errors.is_empty() {
            format!("{} programs; native Rev2 .syx", self.programs.len())
        } else {
            format!(
                "{} programs; rejected {} file(s): {}",
                self.programs.len(),
                errors.len(),
                errors[0]
            )
        };
    }
    fn load(&mut self, i: usize) {
        if let Some(p) = self.programs.get(i) {
            self.state.patch = p.patch;
            self.baseline = p.patch;
            self.compare = None;
            self.program = i;
            self.state.generation = self.state.generation.wrapping_add(1);
            self.status = if (0..2)
                .any(|l| p.patch.data[l * 1024 + 123] > 0 && p.patch.data[l * 1024 + 124] == 16)
            {
                "Chord-memory layout unverified; chord voicing is not reproduced".into()
            } else {
                format!("{} / {}", p.patch.name(0), p.source)
            };
            self.publish();
        }
    }
    fn dirty(&self) -> bool {
        self.state.patch != self.baseline
    }
    fn save(&mut self) -> Result<(), String> {
        self.restore_compare();
        let path = self.save_dir.join(format!(
            "U{}-{:03}.syx",
            self.slot / 128 + 1,
            self.slot % 128 + 1
        ));
        let bytes = self
            .state
            .patch
            .export(Some(((self.slot / 128) as u8, (self.slot % 128) as u8)))?;
        std::fs::create_dir_all(&self.save_dir).map_err(|e| e.to_string())?;
        let temp = path.with_extension("syx.tmp");
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        self.baseline = self.state.patch;
        self.status = format!("Saved {}", path.display());
        Ok(())
    }
    fn restore_compare(&mut self) {
        if let Some(p) = self.compare.take() {
            self.state.patch = p;
        }
    }
    fn compare(&mut self) {
        if let Some(p) = self.compare.take() {
            self.state.patch = p;
        } else {
            self.compare = Some(self.state.patch);
            self.state.patch = self.baseline;
        }
        self.state.generation = self.state.generation.wrapping_add(1);
        self.publish();
    }
    fn copy_layer(&mut self, swap: bool, seq_only: bool) {
        let start = if seq_only { 256 } else { 0 };
        let end = if seq_only { 1024 } else { 1024 };
        for i in start..end {
            if swap {
                self.state.patch.data.swap(i, i + 1024);
            } else {
                let src = self.layer * 1024 + i;
                let dst = (1 - self.layer) * 1024 + i;
                self.state.patch.data[dst] = self.state.patch.data[src];
            }
        }
        self.publish();
    }
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let b = self.layer * 1024;
        let ranges: &[(usize, usize)] = match self.section {
            0 => &[(0, 22)],
            1 => &[(22, 30)],
            2 => &[(30, 53)],
            3 => &[(53, 77)],
            4 => &[(77, 101)],
            5 => &[(101, 111)],
            6 => &[(115, 121)],
            7 => &[(122, 132), (208, 210), (231, 233)],
            8 => &[(132, 140)],
            9 => &[(111, 115), (138, 140), (140, 204)],
            _ => &[],
        };
        for &(start, end) in ranges {
            for p in spec::PARAMS
                .iter()
                .filter(|p| p.offset >= start && p.offset < end)
            {
                let offset = if p.offset == 231 || p.offset == 232 {
                    p.offset
                } else {
                    b + p.offset
                };
                rows.push(Row {
                    name: if (140..204).contains(&p.offset) {
                        format!(
                            "Gate {} step {:02}",
                            (p.offset - 140) / 16 + 1,
                            (p.offset - 140) % 16 + 1
                        )
                    } else {
                        p.name.into()
                    },
                    value: if (53..57).contains(&p.offset)
                        && self.state.patch.data[b + 69 + p.offset - 53] != 0
                    {
                        spec::LFO_STEPS[(self.state.patch.data[offset] as usize / 9).min(15)].into()
                    } else {
                        spec::text(offset, self.state.patch.data[offset])
                    },
                    offset: Some(offset),
                    action: 0,
                });
            }
        }
        if (10..=13).contains(&self.section) {
            let tracks: Vec<usize> = if self.section == 13 {
                vec![self.track]
            } else {
                vec![(self.section - 10) * 2, (self.section - 10) * 2 + 1]
            };
            for t in tracks {
                for s in self.seq_page * 16..self.seq_page * 16 + 16 {
                    let i = b + 256 + t * 128 + s + if self.section == 13 { 64 } else { 0 };
                    rows.push(Row {
                        name: format!("Track {} step {:02}", t + 1, s + 1),
                        value: spec::text(i, self.state.patch.data[i]),
                        offset: Some(i),
                        action: 0,
                    });
                }
            }
        }
        let mut action = |id: usize, name: &str, value: String| {
            rows.push(Row {
                name: name.into(),
                value,
                offset: None,
                action: id,
            })
        };
        if self.section == 14 {
            action(
                1,
                "Program",
                format!("{:04} {}", self.program + 1, self.state.patch.name(0)),
            );
            action(
                2,
                "Save slot",
                format!("U{} {:03}", self.slot / 128 + 1, self.slot % 128 + 1),
            );
            action(3, "Write SysEx", "Press encoder".into());
            action(4, "Rescan .syx", "Press encoder".into());
            action(
                5,
                "Compare",
                if self.compare.is_some() {
                    "Original"
                } else {
                    "Edited"
                }
                .into(),
            );
            action(6, "Copy layer", "To other layer".into());
            action(7, "Swap layers", "A / B".into());
            action(8, "Copy poly sequence", "To other layer".into());
            action(9, "Swap poly sequences", "A / B".into());
            action(10, "Initialize", "Press encoder".into());
        }
        if self.section == 15 {
            action(11, "Voices", self.state.voices.to_string());
            action(12, "Hold", self.state.hold.to_string());
            action(13, "Pad octave", self.octave.to_string());
            action(14, "Transpose", self.transpose.to_string());
            action(
                15,
                "Follow device clock",
                self.state.follow_clock.to_string(),
            );
            action(16, "Mono output", self.state.mono.to_string());
            action(17, "Breath", format!("{:.0}", self.state.breath * 127.));
            action(18, "Foot", format!("{:.0}", self.state.foot * 127.));
            action(
                19,
                "Expression",
                format!("{:.0}", self.state.expression * 127.),
            );
            action(20, "Poly step recording", self.record.to_string());
            action(21, "Record step", (self.record_step + 1).to_string());
            action(
                22,
                "Alternative tuning",
                tuning::NAMES[self.tuning_index].into(),
            );
            action(23, "Master coarse", self.state.master_coarse.to_string());
            action(24, "Master fine cents", self.state.master_fine.to_string());
            action(25, "Scan MIDI inputs", "Press encoder".into());
            action(26, "Direct MIDI input", self.midi.name());
            action(
                27,
                "MIDI channel",
                if self.midi_channel == 0 {
                    "All".into()
                } else {
                    self.midi_channel.to_string()
                },
            );
            action(28, "Multi mode", self.state.multi.to_string());
        }
        rows
    }
    fn edit(&mut self, d: i32, press: bool) {
        let rows = self.rows();
        let Some(row) = rows.get(self.selected).cloned() else {
            return;
        };
        self.restore_compare();
        if let Some(i) = row.offset {
            if i >= patch::RAW_LEN {
                self.status = "Rev2 SysEx omits these two velocities".into();
                return;
            }
            let v = (self.state.patch.data[i] as i32 + d)
                .clamp(spec::minimum(i) as i32, spec::maximum(i) as i32) as u8;
            let _ = self.state.patch.set(i, v);
        } else {
            match row.action {
                1 => {
                    let n =
                        (self.program as i32 + d).rem_euclid(self.programs.len() as i32) as usize;
                    self.load(n);
                }
                2 => self.slot = (self.slot as i32 + d).rem_euclid(512) as usize,
                3 if press => {
                    if let Err(e) = self.save() {
                        self.status = format!("Save failed: {e}");
                    }
                }
                4 if press => self.scan(),
                5 if press => self.compare(),
                6 if press => self.copy_layer(false, false),
                7 if press => self.copy_layer(true, false),
                8 if press => self.copy_layer(false, true),
                9 if press => self.copy_layer(true, true),
                10 if press => {
                    self.state.patch = Patch::default();
                    self.state.generation = self.state.generation.wrapping_add(1);
                }
                11 => self.state.voices = if self.state.voices == 16 { 8 } else { 16 },
                12 => {
                    self.state.hold = !self.state.hold;
                    if !self.state.hold {
                        self.held = [0; 128];
                    }
                }
                13 => self.octave = (self.octave + d).clamp(-3, 3),
                14 => self.transpose = (self.transpose + d).clamp(-12, 12),
                15 => self.state.follow_clock = !self.state.follow_clock,
                16 => self.state.mono = !self.state.mono,
                17 => self.state.breath = (self.state.breath + d as f32 / 127.).clamp(0., 1.),
                18 => self.state.foot = (self.state.foot + d as f32 / 127.).clamp(0., 1.),
                19 => {
                    self.state.expression = (self.state.expression + d as f32 / 127.).clamp(0., 1.)
                }
                20 => self.record = !self.record,
                21 => self.record_step = (self.record_step as i32 + d).rem_euclid(64) as usize,
                22 => {
                    self.tuning_index = (self.tuning_index as i32 + d).rem_euclid(17) as usize;
                    self.state.tuning = tuning::table(self.tuning_index);
                }
                23 => {
                    self.state.master_coarse =
                        (self.state.master_coarse + d as f32).clamp(-12., 12.)
                }
                24 => self.state.master_fine = (self.state.master_fine + d as f32).clamp(-50., 50.),
                25 if press => {
                    if let Err(e) = self.midi.scan() {
                        self.status = e;
                    }
                }
                26 => {
                    let n = (self.midi.selected as i32 + d)
                        .rem_euclid(self.midi.names.len() as i32 + 1)
                        as usize;
                    if let Err(e) = self.midi.select(n) {
                        self.status = e;
                    }
                    self.midi_keys = [[0; 128]; 2];
                    self.state.midi_notes = [[0; 128]; 2];
                }
                27 => self.midi_channel = (self.midi_channel as i32 + d).rem_euclid(17) as usize,
                28 => self.state.multi = !self.state.multi,
                _ => {}
            }
        }
        self.publish();
    }
    fn keys(&mut self, input: &Input) {
        let mut notes = [0; 128];
        for (n, v) in input.midi_keys.0.iter().enumerate() {
            if *v > 0 {
                let n = (n as i32 + self.transpose).clamp(0, 127) as usize;
                notes[n] = *v;
            }
        }
        for (i, on) in input.grid.iter().enumerate() {
            if *on {
                let n = (crate::apps::mi_kit::pad_note(i, self.octave) as i32 + self.transpose)
                    .clamp(0, 127) as usize;
                let n = n.min(127);
                notes[n] = if input.pad_pressure[i] > 0. {
                    (input.pad_pressure[i] * 127.).clamp(1., 127.) as u8
                } else {
                    100
                };
            }
        }
        if let Some(n) = self.pointer {
            notes[n as usize] = 100;
        }
        if self.state.hold {
            for n in 0..128 {
                self.held[n] = self.held[n].max(notes[n]);
            }
            notes = self.held;
        }
        if self.record {
            let fresh = (0..128).any(|n| notes[n] > 0 && self.state.notes[n] == 0);
            if fresh {
                let b = self.layer * 1024;
                let mut track = 0;
                for t in 0..6 {
                    self.state.patch.data[b + 256 + t * 128 + self.record_step] = 0;
                    self.state.patch.data[b + 320 + t * 128 + self.record_step] = 128;
                }
                for n in 0..128 {
                    if notes[n] > 0 && track < 6 {
                        self.state.patch.data[b + 256 + track * 128 + self.record_step] = n as u8;
                        self.state.patch.data[b + 320 + track * 128 + self.record_step] =
                            notes[n].min(127).saturating_add(128);
                        track += 1;
                    }
                }
                self.record_step = (self.record_step + 1) % 64;
            }
        }
        self.state.notes = notes;
        self.state.bend = (input.pitch_bend + self.midi_bend).clamp(-1., 1.);
        self.state.wheel = input.mod_wheel.max(self.midi_wheel);
        self.state.pressure = input.aftertouch.max(self.midi_pressure);
        self.publish();
    }
    fn receive(&mut self, msg: &[u8]) {
        if msg.first() == Some(&0xf0) {
            match patch::import(msg) {
                Ok(patches) => {
                    for p in patches {
                        self.programs.push(Program {
                            patch: p,
                            source: "Live MIDI".into(),
                        });
                    }
                    self.load(self.programs.len() - 1);
                    self.status = "Received Rev2 SysEx".into();
                }
                Err(e) => self.status = e,
            }
            return;
        }
        let Some(event) = self.decoder.message(msg) else {
            return;
        };
        let channel = match event {
            midi::Event::Note(c, ..)
            | midi::Event::Control(c, ..)
            | midi::Event::Bend(c, ..)
            | midi::Event::Pressure(c, ..)
            | midi::Event::Program(c, ..) => Some(c as usize + 1),
            _ => None,
        };
        if let Some(c) = channel {
            if self.midi_channel > 0
                && c != self.midi_channel
                && !(self.state.multi && c == self.midi_channel % 16 + 1)
            {
                return;
            }
        }
        let layer = if self.state.multi && channel == Some(self.midi_channel % 16 + 1) {
            1
        } else {
            0
        };
        match event {
            midi::Event::Note(_, n, v) => {
                self.midi_keys[layer][n as usize] = v;
                if v > 0 || !self.sustain {
                    self.state.midi_notes[layer][n as usize] = v;
                }
            }
            midi::Event::Bend(_, v) => self.midi_bend = v,
            midi::Event::Pressure(_, v) => self.midi_pressure = v as f32 / 127.,
            midi::Event::Control(_, cc, v) => match cc {
                1 => self.midi_wheel = v as f32 / 127.,
                2 => self.state.breath = v as f32 / 127.,
                4 => self.state.foot = v as f32 / 127.,
                11 => self.state.expression = v as f32 / 127.,
                7 => self.shared.level.set(v as f32 / 127.),
                64 => {
                    self.sustain = v >= 64;
                    if !self.sustain {
                        self.state.midi_notes = self.midi_keys;
                    }
                }
                120 | 123 => {
                    self.midi_keys = [[0; 128]; 2];
                    self.state.midi_notes = [[0; 128]; 2];
                }
                _ => {
                    if let Some(i) = midi::cc_offset(cc) {
                        let offset = if i == 231 || i == 232 {
                            i
                        } else {
                            self.layer * 1024 + i
                        };
                        let max = spec::parameter(i).map_or(255, |p| p.max);
                        let _ = self.state.patch.set(offset, v.min(max));
                    }
                }
            },
            midi::Event::Parameter(n, v) => {
                if let Some(offset) = spec::nrpn_offset(n) {
                    let _ = self.state.patch.set(offset, v.min(255) as u8);
                } else {
                    match n {
                        4096 => self.state.master_fine = (v as f32 - 50.).clamp(-50., 50.),
                        4097 => self.state.master_coarse = (v as f32 - 12.).clamp(-12., 12.),
                        4098 => self.midi_channel = (v as usize).min(16),
                        4099 => self.state.follow_clock = v >= 2,
                        4115 => self.state.mono = v > 0,
                        4116 => {
                            self.tuning_index = (v as usize).min(16);
                            self.state.tuning = tuning::table(self.tuning_index);
                        }
                        4119 => self.state.multi = v > 0,
                        4190 => self.layer = (v as usize).min(1),
                        1088 => self.state.playing = v > 0,
                        16383 => self.record = v > 0,
                        _ => {}
                    }
                }
            }
            midi::Event::Program(_, bank, number) => {
                if let Some(i) = self
                    .programs
                    .iter()
                    .position(|p| p.patch.location == Some((bank, number)))
                {
                    self.load(i);
                }
            }
            midi::Event::Start => self.state.playing = true,
            midi::Event::Stop => self.state.playing = false,
            midi::Event::Clock => {}
        }
        if matches!(
            event,
            midi::Event::Clock | midi::Event::Start | midi::Event::Stop
        ) {
            crate::clock::Clock::shared().midi_message(msg);
        }
        let input = self.input;
        self.keys(&input);
    }
}
impl App for ProphetApp {
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        if index < self.rows().len() {
            let keep = std::mem::replace(&mut self.selected, index);
            self.edit(delta, false);
            self.selected = keep;
        }
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.shared.tail.load(Ordering::Relaxed) || self.state.playing
    }
    fn on_exit(&mut self) {
        self.pointer = None;
        if !self.state.hold {
            self.state.notes = [0; 128];
        }
        self.publish();
    }
    fn running(&self) -> Option<bool> {
        Some(self.state.playing)
    }
    fn toggle_running(&mut self) {
        self.state.playing = !self.state.playing;
        self.publish();
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some("REV2")
    }
    fn toggle_grid_mode(&mut self) {
        self.section = if self.section == 14 { 0 } else { 14 };
        self.selected = 0;
    }
    fn hint(&self) -> String {
        "Pads: keys | L1: A/B | R1: section | encoders: select/edit | F3: sequence".into()
    }
    fn tick(&mut self, input: &Input) {
        self.input = *input;
        if input.shoulder_press[0] {
            self.layer = 1 - self.layer;
            self.selected = 0;
        }
        if input.shoulder_press[1] {
            self.section = (self.section + 1) % 16;
            self.selected = 0;
        }
        let n = self.rows().len();
        let nav = input.knob1 + input.navigation_steps;
        if n > 0 && nav != 0 {
            self.selected = (self.selected as i32 + nav).rem_euclid(n as i32) as usize;
        }
        if input.knob2 != 0 {
            self.edit(input.knob2, false);
        }
        if input.knob2_press {
            self.edit(0, true);
        }
        if input.knob1_press {
            self.compare();
        }
        self.keys(input);
    }
    fn background_tick(&mut self) {
        for _ in 0..256 {
            let Some(msg) = self.midi.pop() else {
                break;
            };
            self.receive(&msg);
        }
        if self.midi.overflowed() {
            self.status = "MIDI queue overflow: reconnect input".into();
            self.midi_keys = [[0; 128]; 2];
            self.state.midi_notes = [[0; 128]; 2];
            self.publish();
        }
    }
    fn draw(&mut self, fb: &mut FrameBuffer) {
        panel::draw(self, fb);
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let rows = self.rows();
        let page = self.selected / 4;
        let shown: Vec<_> = rows.iter().skip(page * 4).take(4).collect();
        SlintExtra::Rev2(crate::app::Rev2Extra {
            name: self.state.patch.name(self.layer),
            source: self.programs[self.program].source.clone(),
            status: self.status.clone(),
            sections: SECTIONS.iter().map(|s| s.to_string()).collect(),
            section: self.section,
            layer: self.layer,
            page,
            pages: (rows.len() + 3) / 4,
            sequence_page: self.seq_page,
            velocity_track: self.track,
            selected: self.selected % 4,
            labels: shown.iter().map(|r| r.name.clone()).collect(),
            values: shown.iter().map(|r| r.value.clone()).collect(),
            norms: shown
                .iter()
                .map(|r| {
                    r.offset.map_or(-1., |i| {
                        let min = spec::minimum(i) as f32;
                        (self.state.patch.data[i] as f32 - min)
                            / (spec::maximum(i) as f32 - min).max(1.)
                    })
                })
                .collect(),
            keys: (48..72)
                .map(|n| {
                    self.state.notes[(n + self.octave * 12 + self.transpose).clamp(0, 127) as usize]
                        > 0
                })
                .collect(),
            dirty: self.dirty(),
            comparing: self.compare.is_some(),
            peak: self.shared.peak.get(),
        })
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        match x as i32 {
            1000..=1015 => {
                self.section = x as usize - 1000;
                self.selected = 0;
            }
            1100..=1103 => {
                let row = self.selected / 4 * 4 + x as usize - 1100;
                if row < self.rows().len() {
                    self.selected = row;
                    self.edit(y.round() as i32, y == 0.);
                }
            }
            1200 => self.load(
                (self.program as i32 + y as i32).rem_euclid(self.programs.len() as i32) as usize,
            ),
            1201 => {
                self.section = 14;
                self.selected = 0;
            }
            1202 => self.compare(),
            1203 => {
                self.restore_compare();
                self.state.patch = Patch::default();
                self.state.generation = self.state.generation.wrapping_add(1);
                self.publish();
            }
            1204 => {
                if let Err(e) = self.save() {
                    self.status = format!("Save failed: {e}");
                }
            }
            1205 => {
                self.layer = 1 - self.layer;
                self.selected = 0;
            }
            1206 => {
                self.section = 15;
                self.selected = 0;
            }
            1207 => {
                self.seq_page = (self.seq_page + 1) % 4;
                self.selected = 0;
            }
            1208 => {
                self.track = (self.track + 1) % 6;
                self.selected = 0;
            }
            1300..=1323 => {
                self.pointer = if y > 0. {
                    Some(
                        (48 + x as i32 - 1300 + self.octave * 12 + self.transpose).clamp(0, 127)
                            as u8,
                    )
                } else {
                    None
                };
                let input = self.input;
                self.keys(&input);
            }
            1400 => {
                let pages = (self.rows().len() + 3) / 4;
                if pages > 0 {
                    self.selected = (((self.selected / 4) as i32 + y as i32)
                        .rem_euclid(pages as i32) as usize)
                        * 4;
                }
            }
            _ => panel::pointer(self, x, y),
        }
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
            .into_iter()
            .map(|r| (r.name, r.value, false))
            .collect()
    }
    fn slint_selected(&self) -> usize {
        self.selected
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(dsp::Processor::new(self.shared.clone())))
    }
}
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    Box::new(ProphetApp::new(
        ctx.get::<ModBus>(),
        ctx.get::<AudioBus>(),
        ctx.get::<MixerBus>(),
    ))
}
