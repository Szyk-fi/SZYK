//! Chip Player: plays the chip-music file formats of old consoles and
//! computers with Game_Music_Emu (vendor/gme, LGPL-2.1) -- real emulations of
//! the sound hardware running the files' own code, not recordings.
//!
//! NSF/NSFE (NES, with VRC6/VRC7/FDS/MMC5/Namco/Sunsoft expansions), SPC
//! (SNES), GBS (Game Boy), VGM/VGZ and GYM (Mega Drive/Genesis, Master System,
//! and the arcade chips VGM logs), HES (PC Engine), KSS (MSX), AY (ZX
//! Spectrum/Amstrad) and SAP (Atari). Drop files in `chiptunes/` (see its
//! README); nothing there is committed.
//!
//! Pick a Song, step through its Tracks (an NSF holds a whole soundtrack), and
//! F3 plays or pauses. The pads *are* the chip's voices: tap one to mute or
//! unmute that channel (pulse 1, triangle, noise... whatever the format's chip
//! has), so a track can be taken apart by ear. Tempo changes speed without
//! changing pitch, Stereo adds the emulator's echo depth, and Treble and Bass
//! are its equalizer. Repeat plays a track again, continues to the next one,
//! or stops at the end; tracks that don't say how long they run play for 2.5
//! minutes and fade.
//!
//! A song is loaded on the UI thread and handed to the audio thread through a
//! try_lock slot, with the finished one handed back the same way, so the audio
//! callback never loads, allocates or frees.

use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::ffi::{c_char, c_int, c_long, c_void, CStr};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Chip Player";

/// Where songs live, anchored at the crate root like `samples/`.
const CHIPTUNES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/chiptunes");

const EXTENSIONS: [&str; 11] = ["nsf", "nsfe", "spc", "gbs", "vgm", "vgz", "gym", "hes", "kss", "ay", "sap"];

// Own palette: CRT phosphor on near-black, a hot-pink accent.
const BG: Rgb565 = Rgb565::new(2, 2, 5);
const INK: Rgb565 = Rgb565::new(24, 56, 30);
const ACCENT: Rgb565 = Rgb565::new(31, 18, 20);
const DIM: Rgb565 = Rgb565::new(10, 20, 16);
const FAINT: Rgb565 = Rgb565::new(4, 6, 8);

/// Tracks without a stated length play this long, then fade.
const FADE_MS: c_int = 6_000;
const MAX_VOICES: usize = 32;
const CHUNK: usize = 1024;

#[allow(non_camel_case_types)]
type Music_Emu = c_void;

#[repr(C)]
struct GmeInfo {
    length: c_int,
    intro_length: c_int,
    loop_length: c_int,
    play_length: c_int,
    fade_length: c_int,
    _reserved_ints: [c_int; 11],
    system: *const c_char,
    game: *const c_char,
    song: *const c_char,
    author: *const c_char,
    copyright: *const c_char,
    comment: *const c_char,
    dumper: *const c_char,
    _reserved_strings: [*const c_char; 9],
}

#[repr(C)]
struct GmeEqualizer {
    treble: f64,
    bass: f64,
    _reserved: [f64; 8],
}

unsafe extern "C" {
    fn gme_open_data(data: *const c_void, size: c_long, out: *mut *mut Music_Emu, sample_rate: c_int) -> *const c_char;
    fn gme_delete(emu: *mut Music_Emu);
    fn gme_track_count(emu: *const Music_Emu) -> c_int;
    fn gme_start_track(emu: *mut Music_Emu, index: c_int) -> *const c_char;
    fn gme_play(emu: *mut Music_Emu, count: c_int, out: *mut i16) -> *const c_char;
    fn gme_track_ended(emu: *const Music_Emu) -> c_int;
    fn gme_tell(emu: *const Music_Emu) -> c_int;
    fn gme_set_fade(emu: *mut Music_Emu, start_msec: c_int);
    fn gme_track_info(emu: *const Music_Emu, out: *mut *mut GmeInfo, track: c_int) -> *const c_char;
    fn gme_free_info(info: *mut GmeInfo);
    fn gme_voice_count(emu: *const Music_Emu) -> c_int;
    fn gme_voice_name(emu: *const Music_Emu, i: c_int) -> *const c_char;
    fn gme_mute_voices(emu: *mut Music_Emu, mask: c_int);
    fn gme_set_tempo(emu: *mut Music_Emu, tempo: f64);
    fn gme_set_stereo_depth(emu: *mut Music_Emu, depth: f64);
    fn gme_set_equalizer(emu: *mut Music_Emu, eq: *const GmeEqualizer);
    fn gme_ignore_silence(emu: *mut Music_Emu, ignore: c_int);
}

/// What the file says about one track.
#[derive(Clone, Default)]
struct TrackInfo {
    system: String,
    game: String,
    song: String,
    author: String,
    copyright: String,
    /// Milliseconds to play before the fade starts.
    play_ms: i32,
}

/// A loaded song, positioned at one track. All allocation is in `open`.
struct Player {
    emu: *mut Music_Emu,
    track: usize,
    tracks: usize,
    voices: Vec<String>,
    info: TrackInfo,
    rate: i32,
    /// Settings last pushed to the emulator, so only changes are sent.
    sent: Sent,
}

#[derive(Clone, Copy, PartialEq)]
struct Sent {
    mute_mask: i32,
    tempo: i32,
    stereo: i32,
    treble: i32,
    bass: i32,
}

const UNSENT: Sent = Sent { mute_mask: i32::MIN, tempo: i32::MIN, stereo: i32::MIN, treble: i32::MIN, bass: i32::MIN };

// An emulator is only ever used by whoever holds the box.
unsafe impl Send for Player {}

fn cstr(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().trim().to_string() }
}

/// `.vgz` (and any gzip) files are gunzipped here; GME is built without zlib.
fn maybe_gunzip(data: Vec<u8>) -> Vec<u8> {
    if data.len() > 2 && data[0] == 0x1f && data[1] == 0x8b {
        use std::io::Read;
        let mut out = Vec::new();
        if flate2::read::GzDecoder::new(&data[..]).take(64 << 20).read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    data
}

impl Player {
    fn open(data: &[u8], track: usize, rate: i32) -> Result<Player, String> {
        let mut emu: *mut Music_Emu = std::ptr::null_mut();
        let err = unsafe { gme_open_data(data.as_ptr() as *const c_void, data.len() as c_long, &mut emu, rate) };
        if !err.is_null() || emu.is_null() {
            return Err(if err.is_null() { "could not open".into() } else { cstr(err) });
        }
        let tracks = unsafe { gme_track_count(emu) }.max(1) as usize;
        let track = track.min(tracks - 1);
        let err = unsafe { gme_start_track(emu, track as c_int) };
        if !err.is_null() {
            unsafe { gme_delete(emu) };
            return Err(cstr(err));
        }
        let voice_count = (unsafe { gme_voice_count(emu) }.max(0) as usize).min(MAX_VOICES);
        let voices = (0..voice_count).map(|i| cstr(unsafe { gme_voice_name(emu, i as c_int) })).collect();
        let mut info = TrackInfo { play_ms: 150_000, ..Default::default() };
        let mut raw: *mut GmeInfo = std::ptr::null_mut();
        if unsafe { gme_track_info(emu, &mut raw, track as c_int) }.is_null() && !raw.is_null() {
            let r = unsafe { &*raw };
            info = TrackInfo { system: cstr(r.system), game: cstr(r.game), song: cstr(r.song), author: cstr(r.author), copyright: cstr(r.copyright), play_ms: r.play_length.max(1_000) };
            unsafe { gme_free_info(raw) };
        }
        unsafe {
            gme_set_fade(emu, info.play_ms);
            // Chip tracks open with silence more often than a silent glitch.
            gme_ignore_silence(emu, 1);
        }
        Ok(Player { emu, track, tracks, voices, info, rate, sent: UNSENT })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe { gme_delete(self.emu) };
    }
}

fn is_song(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Songs in `dir` and one folder level down, sorted by display name.
fn scan(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut walk = |d: &Path, prefix: &str| {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if is_song(&p) {
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                out.push((format!("{prefix}{stem}"), p));
            }
        }
    };
    walk(dir, "");
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            walk(&p, &format!("{name}/"));
        }
    }
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "chip_player",
        layers: vec![Layer::Native(0, "VOICES"), Layer::Controls, Layer::Moments],
        hero: vec![[C_SONG, C_TRACK], [C_VOLUME, C_TEMPO], [C_STEREO, C_REPEAT], [C_TREBLE, C_BASS]],
        browse: Some(C_SONG),
        routes: Routes { stick_x: Some(C_TEMPO), stick_y: Some(C_TREBLE), hand_l: Some(C_VOLUME), hand_r: Some(C_STEREO) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

const C_SONG: usize = 0;
const C_TRACK: usize = 1;
const C_VOLUME: usize = 2;
const C_TEMPO: usize = 3;
const C_STEREO: usize = 4;
const C_REPEAT: usize = 5;
const C_TREBLE: usize = 6;
const C_BASS: usize = 7;
const N_CONTROLS: usize = 8;

const REPEAT: [&str; 3] = ["Track", "All", "Off"];

struct Shared {
    volume: AtomicF32,
    /// 0.5x..2x, stored as a 0..1 position (0.5 = normal speed).
    tempo: AtomicF32,
    stereo: AtomicF32,
    /// 0..1 positions of the emulator's treble (-50..+5 dB) and bass (16000..1 Hz) controls.
    treble: AtomicF32,
    bass: AtomicF32,
    repeat: AtomicUsize,
    running: AtomicBool,
    /// Voices muted, one bit each (the pads).
    muted: AtomicU32,
    cv: [Arc<AtomicF32>; 3],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    peak: AtomicF32,
    rate: AtomicF32,
    elapsed_ms: AtomicI32,
    /// Set by the audio thread when the track reached its end.
    ended: AtomicBool,
    incoming: Mutex<Option<Box<Player>>>,
    retired: Mutex<Vec<Box<Player>>>,
}

pub struct ChipPlayerApp {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    files: Vec<(String, PathBuf)>,
    song: usize,
    track: usize,
    tracks: usize,
    voices: Vec<String>,
    info: TrackInfo,
    loaded_rate: i32,
    status: String,
    /// Raw bytes of the loaded song, kept so a track change doesn't re-read the disk.
    data: Vec<u8>,
    pads_down: [bool; 16],
}

impl ChipPlayerApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dir(Path::new(CHIPTUNES_DIR), sensitivity, nav, mods, bus, mixer)
    }

    pub fn with_dir(dir: &Path, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let mut app = Self {
            p: Arc::new(Shared {
                volume: AtomicF32::new(0.7),
                tempo: AtomicF32::new(0.5),
                stereo: AtomicF32::new(0.0),
                treble: AtomicF32::new(DEFAULT_TREBLE),
                bass: AtomicF32::new(DEFAULT_BASS),
                repeat: AtomicUsize::new(1),
                running: AtomicBool::new(false),
                muted: AtomicU32::new(0),
                cv: [mods.register(format!("{APP_NAME}: Volume")), mods.register(format!("{APP_NAME}: Tempo")), mods.register(format!("{APP_NAME}: Stereo"))],
                mix_level,
                ext_mix_level,
                output,
                peak: AtomicF32::new(0.0),
                rate: AtomicF32::new(48_000.0),
                elapsed_ms: AtomicI32::new(0),
                ended: AtomicBool::new(false),
                incoming: Mutex::new(None),
                retired: Mutex::new(Vec::new()),
            }),
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            files: scan(dir),
            song: 0,
            track: 0,
            tracks: 0,
            voices: Vec::new(),
            info: TrackInfo::default(),
            loaded_rate: 0,
            status: String::new(),
            data: Vec::new(),
            pads_down: [false; 16],
        };
        app.load_song(0);
        app
    }

    /// Reads song `index` from disk and opens its first track.
    fn load_song(&mut self, index: usize) {
        let Some((name, path)) = self.files.get(index).cloned() else {
            self.status = format!("Put chip music in {CHIPTUNES_DIR}");
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                self.data = maybe_gunzip(bytes);
                self.song = index;
                self.open_track(0, &name);
            }
            Err(e) => self.status = format!("Could not read {name}: {e}"),
        }
    }

    /// Opens `track` of the loaded song and queues it for the audio thread.
    fn open_track(&mut self, track: usize, name: &str) {
        let rate = self.p.rate.get() as i32;
        match Player::open(&self.data, track, rate) {
            Ok(player) => {
                self.tracks = player.tracks;
                self.track = player.track;
                self.voices = player.voices.clone();
                self.info = player.info.clone();
                self.loaded_rate = rate;
                self.status.clear();
                self.p.ended.store(false, Ordering::Relaxed);
                self.p.elapsed_ms.store(0, Ordering::Relaxed);
                if let Ok(mut slot) = self.p.incoming.lock() {
                    *slot = Some(Box::new(player));
                }
            }
            Err(e) => {
                self.data.clear();
                self.tracks = 0;
                self.voices.clear();
                self.info = TrackInfo::default();
                self.status = format!("{name}: {e}");
            }
        }
    }

    fn song_name(&self) -> String {
        self.files.get(self.song).map_or_else(|| "(none)".into(), |(n, _)| n.clone())
    }

    fn step_song(&mut self, d: i32) {
        let n = self.files.len() as i32;
        if n > 0 && d != 0 {
            self.load_song((self.song as i32 + d.signum()).rem_euclid(n) as usize);
        }
    }

    fn step_track(&mut self, d: i32) {
        if self.tracks > 0 && d != 0 {
            let t = (self.track as i32 + d.signum()).rem_euclid(self.tracks as i32) as usize;
            let name = self.song_name();
            self.open_track(t, &name);
        }
    }

    fn tempo_factor(&self) -> f32 {
        let p = (self.p.tempo.get() + self.p.cv[1].get()).clamp(0.0, 1.0);
        // 0.5x .. 2x with 1x in the middle.
        2f32.powf(p * 2.0 - 1.0)
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_SONG => ("Song".into(), if self.files.is_empty() { "(none in chiptunes/)".into() } else { format!("{} {}/{}", self.song_name(), self.song + 1, self.files.len()) }),
            C_TRACK => ("Track".into(), if self.tracks == 0 { "-".into() } else { format!("{}/{}", self.track + 1, self.tracks) }),
            C_VOLUME => ("Volume".into(), format!("{:.0}%", (self.p.volume.get() + self.p.cv[0].get()).clamp(0.0, 1.0) * 100.0)),
            C_TEMPO => ("Tempo".into(), format!("{:.2}x", self.tempo_factor())),
            C_STEREO => ("Stereo".into(), format!("{:.0}%", (self.p.stereo.get() + self.p.cv[2].get()).clamp(0.0, 1.0) * 100.0)),
            C_REPEAT => ("Repeat".into(), REPEAT[self.p.repeat.load(Ordering::Relaxed).min(2)].into()),
            C_TREBLE => ("Treble".into(), format!("{:+.0} dB", treble_db(self.p.treble.get()))),
            _ => ("Bass".into(), format!("{:.0} Hz", bass_hz(self.p.bass.get()))),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N_CONTROLS).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn edit_continuous(&mut self, i: usize, d: i32) {
        let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        let bump = |a: &AtomicF32| a.set((a.get() + step).clamp(0.0, 1.0));
        match i {
            C_VOLUME => bump(&self.p.volume),
            C_TEMPO => bump(&self.p.tempo),
            C_STEREO => bump(&self.p.stereo),
            C_TREBLE => bump(&self.p.treble),
            C_BASS => bump(&self.p.bass),
            _ => {}
        }
    }

    fn voice_muted(&self, v: usize) -> bool {
        self.p.muted.load(Ordering::Relaxed) & (1 << v) != 0
    }

    fn toggle_voice(&mut self, v: usize) {
        if v < self.voices.len() {
            self.p.muted.fetch_xor(1 << v, Ordering::Relaxed);
        }
    }
}

/// Game_Music_Emu's own default equalizer: treble -8 dB, bass corner 90 Hz.
const DEFAULT_TREBLE: f32 = (-8.0 + 50.0) / 55.0;
const DEFAULT_BASS: f32 = 0.4636; // ln(90) / ln(16000)

/// The emulator's treble control is -50 (muffled) to +5 (crisp) dB.
fn treble_db(p: f32) -> f32 {
    -50.0 + p.clamp(0.0, 1.0) * 55.0
}

/// Its bass control is a corner in Hz: 1 is full bass, 16000 almost none. The
/// knob runs from the full end up, logarithmically.
fn bass_hz(p: f32) -> f32 {
    16_000f32.powf(p.clamp(0.0, 1.0)).max(1.0)
}

impl PlayHost for ChipPlayerApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).1
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match i {
            C_SONG => if self.files.len() > 1 { self.song as f32 / (self.files.len() - 1) as f32 } else { 0.0 },
            C_TRACK => if self.tracks > 1 { self.track as f32 / (self.tracks - 1) as f32 } else { 0.0 },
            C_VOLUME => self.p.volume.get(),
            C_TEMPO => self.p.tempo.get(),
            C_STEREO => self.p.stereo.get(),
            C_REPEAT => self.p.repeat.load(Ordering::Relaxed) as f32 / 2.0,
            C_TREBLE => self.p.treble.get(),
            C_BASS => self.p.bass.get(),
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_SONG | C_TRACK | C_REPEAT)
    }
    fn kit_pads_play(&self, _layer: u8) -> bool {
        // The pads mute voices; pressing harder means nothing.
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            C_SONG => self.step_song(delta),
            C_TRACK => self.step_track(delta),
            C_REPEAT => {
                if delta != 0 {
                    self.p.repeat.store((self.p.repeat.load(Ordering::Relaxed) as i32 + delta.signum()).rem_euclid(3) as usize, Ordering::Relaxed);
                }
            }
            _ => self.edit_continuous(i, delta),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_TRACK => {
                let name = self.song_name();
                self.open_track(0, &name);
            }
            C_VOLUME => self.p.volume.set(0.7),
            C_TEMPO => self.p.tempo.set(0.5),
            C_STEREO => self.p.stereo.set(0.0),
            C_REPEAT => self.p.repeat.store(1, Ordering::Relaxed),
            C_TREBLE => self.p.treble.set(DEFAULT_TREBLE),
            C_BASS => self.p.bass.set(DEFAULT_BASS),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_SONG => {
                if self.files.len() > 1 {
                    let t = (v * (self.files.len() - 1) as f32).round() as usize;
                    if t != self.song {
                        self.load_song(t);
                    }
                }
            }
            C_TRACK => {
                if self.tracks > 1 {
                    let t = (v * (self.tracks - 1) as f32).round() as usize;
                    if t != self.track {
                        let name = self.song_name();
                        self.open_track(t, &name);
                    }
                }
            }
            C_VOLUME => self.p.volume.set(v),
            C_TEMPO => self.p.tempo.set(v),
            C_STEREO => self.p.stereo.set(v),
            C_REPEAT => self.p.repeat.store((v * 2.0).round() as usize, Ordering::Relaxed),
            C_TREBLE => self.p.treble.set(v),
            C_BASS => self.p.bass.set(v),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        if !self.status.is_empty() {
            self.status.clone()
        } else if !self.p.running.load(Ordering::Relaxed) {
            "paused (F3 plays)".into()
        } else {
            let s = self.p.elapsed_ms.load(Ordering::Relaxed) / 1000;
            format!("{}:{:02} of {}:{:02}", s / 60, s % 60, self.info.play_ms / 60_000, (self.info.play_ms / 1000) % 60)
        }
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        // Pads are numbered bottom-left up, like the other keyboards' ranks.
        let voice = kit::pad_rank(pad);
        self.voices.get(voice).map(|n| n.chars().take(5).collect()).unwrap_or_default()
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, _held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        let voice = kit::pad_rank(pad);
        if voice >= self.voices.len() {
            PadColor::Off
        } else if self.voice_muted(voice) {
            PadColor::Red
        } else {
            PadColor::Green
        }
    }
}

impl App for ChipPlayerApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn running(&self) -> Option<bool> {
        Some(self.p.running.load(Ordering::Relaxed))
    }
    fn toggle_running(&mut self) {
        self.p.running.fetch_xor(true, Ordering::Relaxed);
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.p.running.load(Ordering::Relaxed) { "PAUSE" } else { "PLAY" })
    }
    fn needs_background_audio(&self) -> bool {
        self.p.running.load(Ordering::Relaxed)
    }
    fn tick(&mut self, input: &Input) {
        if let Ok(mut r) = self.p.retired.try_lock() {
            r.clear();
        }
        // The device may run at another rate than the one a song was opened at.
        let rate = self.p.rate.get() as i32;
        if self.loaded_rate != 0 && (rate - self.loaded_rate).abs() > 1 && !self.data.is_empty() {
            let name = self.song_name();
            self.open_track(self.track, &name);
        }
        // The track ran out: what happens next is Repeat's decision.
        if self.p.ended.swap(false, Ordering::Relaxed) {
            match self.p.repeat.load(Ordering::Relaxed) {
                0 => {
                    let name = self.song_name();
                    self.open_track(self.track, &name);
                }
                1 => {
                    if self.track + 1 < self.tracks {
                        self.step_track(1);
                    } else if self.files.len() > 1 {
                        self.step_song(1);
                    } else {
                        let name = self.song_name();
                        self.open_track(0, &name);
                    }
                }
                _ => self.p.running.store(false, Ordering::Relaxed),
            }
        }
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        // On the VOICES layer, a pad tap mutes or unmutes that chip voice.
        if step.native.is_some() {
            for pad in 0..16 {
                if step.input.grid[pad] && !self.pads_down[pad] {
                    self.toggle_voice(kit::pad_rank(pad));
                }
                self.pads_down[pad] = step.input.grid[pad];
            }
        } else {
            self.pads_down = [false; 16];
        }
        if step.menu {
            let input = &step.input;
            self.list.navigate_input(input, N_CONTROLS, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS - 1);
            self.kit_edit(sel, input.knob2);
            if input.knob2_press {
                self.kit_reset(sel);
            }
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
        if self.kit.menu {
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, r.len(), &r, BG, DIM, ACCENT);
        } else if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 280, pal);
        }
        let ink = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        let mut y = 60;
        for (text, style) in [
            (self.info.game.clone(), ink),
            (self.info.song.clone(), MonoTextStyle::new(&SPLEEN_6X12, ACCENT)),
            (self.info.author.clone(), dim),
            (self.info.system.clone(), dim),
            (self.info.copyright.clone(), dim),
        ] {
            if !text.is_empty() {
                Text::new(&clip(&text, 38), Point::new(384, y), style).draw(f).ok();
                y += 16;
            }
        }
        if !self.status.is_empty() {
            Text::new(&clip(&self.status, 38), Point::new(384, y), dim).draw(f).ok();
        }
        // The chip's voices: lit while audible, struck out when muted.
        for (i, name) in self.voices.iter().enumerate().take(12) {
            let style = if self.voice_muted(i) { dim } else { ink };
            let mark = if self.voice_muted(i) { "x" } else { "*" };
            Text::new(&clip(&format!("{mark} {name}"), 38), Point::new(384, 160 + i as i32 * 12), style).draw(f).ok();
        }
        let level = self.p.peak.get().clamp(0.0, 1.0).sqrt();
        Rectangle::new(Point::new(384, 316), Size::new(236, 8)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
        Rectangle::new(Point::new(385, 317), Size::new((level * 234.0) as u32, 6)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        Some(Box::new(Processor { p: Arc::clone(&self.p), player: None, stash: None, buf: [0; CHUNK * 2] }))
    }
}

struct Processor {
    p: Arc<Shared>,
    player: Option<Box<Player>>,
    stash: Option<Box<Player>>,
    buf: [i16; CHUNK * 2],
}

// The emulator is only ever touched from the audio thread once it is here.
unsafe impl Send for Processor {}

impl Processor {
    fn collect_new_player(&mut self) {
        if let Some(old) = self.stash.take() {
            match self.p.retired.try_lock() {
                Ok(mut r) => r.push(old),
                Err(_) => self.stash = Some(old),
            }
        }
        if self.stash.is_some() {
            return;
        }
        let Ok(mut slot) = self.p.incoming.try_lock() else { return };
        if let Some(new) = slot.take() {
            self.stash = self.player.replace(new);
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        self.p.rate.set(rate);
        self.collect_new_player();
        let frames = out.len() / channels;
        let running = self.p.running.load(Ordering::Relaxed);
        let Some(pl) = self.player.as_mut() else { return };
        if !running || pl.rate != rate as i32 {
            // Paused, or waiting for the UI to reopen the song at this rate.
            if let Ok(mut b) = self.p.output.try_lock() {
                b.clear();
                b.resize(frames, 0.0);
            }
            return;
        }
        let emu = pl.emu;
        let want = Sent {
            mute_mask: self.p.muted.load(Ordering::Relaxed) as i32,
            tempo: ((2f32.powf(((self.p.tempo.get() + self.p.cv[1].get()).clamp(0.0, 1.0)) * 2.0 - 1.0)) * 1000.0) as i32,
            stereo: ((self.p.stereo.get() + self.p.cv[2].get()).clamp(0.0, 1.0) * 1000.0) as i32,
            treble: (self.p.treble.get() * 1000.0) as i32,
            bass: (self.p.bass.get() * 1000.0) as i32,
        };
        unsafe {
            if want.mute_mask != pl.sent.mute_mask {
                gme_mute_voices(emu, want.mute_mask);
            }
            if want.tempo != pl.sent.tempo {
                gme_set_tempo(emu, want.tempo as f64 / 1000.0);
            }
            if want.stereo != pl.sent.stereo {
                gme_set_stereo_depth(emu, want.stereo as f64 / 1000.0);
            }
            if want.treble != pl.sent.treble || want.bass != pl.sent.bass {
                let eq = GmeEqualizer { treble: treble_db(self.p.treble.get()) as f64, bass: bass_hz(self.p.bass.get()) as f64, _reserved: [0.0; 8] };
                gme_set_equalizer(emu, &eq);
            }
        }
        pl.sent = want;
        let gain = (self.p.volume.get() + self.p.cv[0].get()).clamp(0.0, 1.0);
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * gain * gain * 2.0 / 32768.0;
        let mut peak = 0.0f32;
        let mut bus = self.p.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            let err = unsafe { gme_play(emu, (n * 2) as c_int, self.buf.as_mut_ptr()) };
            if !err.is_null() {
                self.buf[..n * 2].fill(0);
            }
            for k in 0..n {
                let (l, r) = (self.buf[k * 2] as f32 * master, self.buf[k * 2 + 1] as f32 * master);
                peak = peak.max(l.abs().max(r.abs()));
                let frame = &mut out[(done + k) * channels..(done + k + 1) * channels];
                match frame {
                    [a, b, ..] => {
                        *a = l;
                        *b = r;
                    }
                    [a] => *a = (l + r) * 0.5,
                    [] => {}
                }
                if let Some(b) = bus.as_mut() {
                    b.push((l + r) * 0.5);
                }
            }
            done += n;
        }
        self.p.peak.set(peak);
        self.p.elapsed_ms.store(unsafe { gme_tell(emu) }, Ordering::Relaxed);
        if unsafe { gme_track_ended(emu) } != 0 {
            self.p.ended.store(true, Ordering::Relaxed);
        }
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(ChipPlayerApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete, minimal NSF (NES music) whose INIT routine turns on the
    /// APU's first pulse channel at period 253: 1789773 / (16 * 254) = 440 Hz.
    /// `tracks` songs share it, so track stepping can be exercised. Built here
    /// because real soundtracks are user-supplied and never committed.
    fn tiny_nsf(tracks: u8) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend(b"NESM\x1a");
        h.push(1); // version
        h.push(tracks);
        h.push(1); // starting song
        h.extend(0x8000u16.to_le_bytes()); // load
        h.extend(0x8000u16.to_le_bytes()); // init
        h.extend(0x8020u16.to_le_bytes()); // play
        for s in ["Tiny Test", "Portamax", "none"] {
            let mut f = [0u8; 32];
            f[..s.len()].copy_from_slice(s.as_bytes());
            h.extend(f);
        }
        h.extend(16639u16.to_le_bytes()); // NTSC speed
        h.extend([0u8; 8]); // bankswitch
        h.extend(20000u16.to_le_bytes()); // PAL speed
        h.push(0); // NTSC
        h.push(0); // no expansion chips
        h.extend([0u8; 4]);
        assert_eq!(h.len(), 128);
        let mut code = vec![
            0xA9, 0x01, 0x8D, 0x15, 0x40, // LDA #1; STA $4015  (enable pulse 1)
            0xA9, 0xBF, 0x8D, 0x00, 0x40, // LDA #$BF; STA $4000 (duty 50%, constant volume 15)
            0xA9, 0xFD, 0x8D, 0x02, 0x40, // LDA #$FD; STA $4002 (period low)
            0xA9, 0x00, 0x8D, 0x03, 0x40, // LDA #0; STA $4003  (period high, restart)
            0x60, // RTS
        ];
        code.resize(0x20, 0xEA);
        code.push(0x60); // PLAY: RTS
        h.extend(code);
        h
    }

    fn dir_with(files: &[(&str, Vec<u8>)]) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("portamax-chip-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        for (path, data) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        }
        dir
    }

    fn app(dir: &Path) -> ChipPlayerApp {
        ChipPlayerApp::with_dir(dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 512 * 2];
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite()));
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn hz(x: &[f32]) -> f32 {
        let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 / (x.len() as f32 / 48_000.0)
    }

    #[test]
    fn a_nsf_loads_with_its_tags_tracks_and_voices() {
        let dir = dir_with(&[("tiny.nsf", tiny_nsf(3)), ("notes.txt", b"ignored".to_vec())]);
        let a = app(&dir);
        assert_eq!(a.files.len(), 1, "only music files are listed");
        assert_eq!(a.tracks, 3);
        assert_eq!(a.info.game, "Tiny Test");
        assert_eq!(a.info.author, "Portamax");
        assert!(a.voices.len() >= 5, "the NES has pulse, triangle, noise and DPCM voices: {:?}", a.voices);
    }

    #[test]
    fn it_opens_paused_and_silent_and_f3_plays_a_real_nes_tone_at_its_pitch() {
        let dir = dir_with(&[("tiny.nsf", tiny_nsf(1))]);
        let mut a = app(&dir);
        assert_eq!(a.running(), Some(false));
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&render(&mut p, 5)) < 1e-9, "paused is silent");
        a.toggle_running();
        render(&mut p, 4);
        let out = render(&mut p, 40);
        assert!(rms(&out) > 0.02, "the APU sounds ({})", rms(&out));
        let f = hz(&out);
        assert!((f - 440.0).abs() < 6.0, "period 253 is 440 Hz, got {f}");
        a.toggle_running();
        assert!(rms(&render(&mut p, 5)) < 1e-9, "paused again");
    }

    #[test]
    fn muting_a_voice_silences_it_and_the_pad_lights_say_so() {
        let dir = dir_with(&[("tiny.nsf", tiny_nsf(1))]);
        let mut a = app(&dir);
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        render(&mut p, 4);
        assert!(rms(&render(&mut p, 20)) > 0.02);
        let pad = (0..16).find(|&i| kit::pad_rank(i) == 0).unwrap();
        assert_eq!(a.kit_pad_color(0, pad, false), crate::led_output::PadColor::Green);
        a.toggle_voice(0); // pulse 1
        assert_eq!(a.kit_pad_color(0, pad, false), crate::led_output::PadColor::Red);
        // The emulator buffers a few tens of milliseconds, so the cut lands a few blocks late.
        render(&mut p, 8);
        assert!(rms(&render(&mut p, 20)) < 0.002, "pulse 1 muted: nothing else is playing");
        a.toggle_voice(0);
        render(&mut p, 4);
        assert!(rms(&render(&mut p, 20)) > 0.02, "and back");
    }

    #[test]
    fn tempo_changes_speed_not_pitch_and_the_volume_scales_the_output() {
        let dir = dir_with(&[("tiny.nsf", tiny_nsf(1))]);
        let mut a = app(&dir);
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        render(&mut p, 4);
        let base = render(&mut p, 30);
        a.kit_set_norm(C_VOLUME, 0.35);
        render(&mut p, 2);
        let quiet = render(&mut p, 30);
        assert!(rms(&quiet) < rms(&base) * 0.5, "volume turns it down");
        a.kit_set_norm(C_VOLUME, 0.7);
        a.kit_set_norm(C_TEMPO, 1.0);
        render(&mut p, 4);
        let fast = render(&mut p, 30);
        assert!((hz(&fast) - 440.0).abs() < 12.0, "a constant tone keeps its pitch at 2x tempo, got {}", hz(&fast));
    }

    #[test]
    fn tracks_and_songs_step_and_wrap() {
        let dir = dir_with(&[("a.nsf", tiny_nsf(3)), ("b/b.nsf", tiny_nsf(2))]);
        let mut a = app(&dir);
        assert_eq!(a.files.len(), 2);
        a.kit_edit(C_TRACK, -1);
        assert_eq!(a.track, 2, "wraps backwards");
        a.kit_edit(C_SONG, 1);
        assert_eq!(a.song_name(), "b/b");
        assert_eq!(a.tracks, 2);
        assert_eq!(a.track, 0, "a new song starts at its first track");
    }

    #[test]
    fn a_gzipped_file_is_unpacked_and_a_junk_file_is_reported() {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tiny_nsf(1)).unwrap();
        let dir = dir_with(&[("packed.vgz", gz.finish().unwrap())]);
        // A .vgz that isn't VGM inside is the closest honest check of the unpack step:
        // GME must be handed the NSF, not the gzip bytes.
        assert_eq!(maybe_gunzip(std::fs::read(dir.join("packed.vgz")).unwrap())[..4], *b"NESM");
        let dir = dir_with(&[("junk.nsf", b"definitely not music".to_vec())]);
        let a = app(&dir);
        assert!(a.status.contains("junk"), "{}", a.status);
        assert_eq!(a.tracks, 0);
    }

    #[test]
    fn with_no_songs_it_says_so_and_stays_silent() {
        let dir = dir_with(&[]);
        let mut a = app(&dir);
        assert!(a.status.contains("chiptunes"), "{}", a.status);
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&render(&mut p, 5)) < 1e-9);
    }

    #[test]
    fn it_swaps_songs_while_running_and_frees_off_the_audio_thread() {
        let dir = dir_with(&[("a.nsf", tiny_nsf(1)), ("b.nsf", tiny_nsf(1))]);
        let mut a = app(&dir);
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        render(&mut p, 4);
        a.kit_edit(C_SONG, 1);
        render(&mut p, 4);
        assert!(!a.p.retired.lock().unwrap().is_empty(), "the replaced song was handed back, not freed in the callback");
        a.tick(&Input::default());
        assert!(a.p.retired.lock().unwrap().is_empty(), "the UI frees it");
    }

    #[test]
    fn its_knobs_are_mod_inputs_and_it_opens_on_the_play_view() {
        let dir = dir_with(&[]);
        let modbus = Arc::new(ModBus::new());
        let a = ChipPlayerApp::with_dir(&dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&modbus), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        for name in ["Chip Player: Volume", "Chip Player: Tempo", "Chip Player: Stereo", "Mixer: Chip Player Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
        assert!(a.play_column().is_some());
    }
}
