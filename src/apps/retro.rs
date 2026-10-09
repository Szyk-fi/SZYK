//! Portamax Retro -- a multi-console Retro *platform*, not a one-off
//! emulator, built on a genuinely different architecture than the
//! Libretro/dlopen design originally sketched for it.
//!
//! **Why not Libretro's real "dynamically load any `.so` core"
//! model**: that depends on a dynamic linker and a real OS mapping
//! executable pages at runtime. The actual target hardware (an
//! STM32N6 microcontroller -- see `main.rs`'s own doc comment, and
//! `display.rs`'s AMS317PN01 panel) has neither: no OS, no MMU, no
//! `dlopen`. A frontend built around runtime core-loading would work
//! great on this desktop sim and then simply not exist as a concept
//! once ported to firmware -- a dead end, not a simplification to
//! revisit later.
//!
//! **What does translate**: linking every supported system straight
//! into the firmware image at *build* time, the same way
//! `plaits_ffi.rs`/`clouds_ffi.rs` already statically link Mutable
//! Instruments' real DSP code into this binary instead of loading it
//! from disk -- then picking *which one* to run with a plain in-
//! firmware `match` at runtime (the `Console` menu row below), not a
//! ROM-drive full of `.so` files. Compiling in NES/SNES/dozens of
//! arcade boards and branching on an enum is completely ordinary bare-
//! metal code; it's dynamically *loading* a core from storage that
//! doesn't translate, not having more than one statically linked in.
//!
//! **Why not real MAME for "Arcade"**: MAME itself is a multi-million-
//! line C++ codebase built around a dynamic driver-loading model and
//! desktop-class resource assumptions -- the same "dead end on real
//! hardware" problem Libretro's dlopen model has above, just at a much
//! larger scale, and not something any amount of vendoring effort
//! changes. What's actually wired in for `Console::Arcade` is
//! [`phosphor-machines`](https://github.com/patsoffice/phosphor-emulator),
//! a genuinely different thing: a from-scratch, actively developed,
//! pure-Rust re-implementation of dozens of *specific* classic boards
//! (Pac-Man, Galaga, Donkey Kong, Dig Dug, Frogger, Joust, Robotron,
//! Defender/Williams hardware, Asteroids, Tempest, Xevious, and more --
//! see `phosphor_machines::registry::all()`), each a small, well-
//! documented, real 8-bit-era board -- exactly the same "one well-
//! defined system, not a sprawling generalist emulator" scope this
//! project already applied to NES and SNES individually. Real
//! precedent exists for this *class* of hardware on MCU-class chips
//! (e.g. the `galagino` project running Galaga/Pac-Man/Donkey Kong on
//! an ESP32), which is a meaningfully better sign for real-hardware
//! feasibility than SNES got -- though, same caveat as SNES, actually
//! unverified on the real STM32N6 board.
//!
//! **Why NES came first, then SNES, then Arcade**: full 65816+PPU+APU
//! (SNES-class) emulation is a heavy workload that struggles even on
//! 300MHz+ ARM9/ARM11-class chips from 2000s-2010s handhelds -- an
//! STM32N6's Cortex-M55 (no MMU, on-chip RAM in the single-digit-MB
//! range, an NPU that doesn't help general CPU emulation) is a much
//! harder target for it than for NES or the individually much simpler
//! 8-bit arcade boards. Whether SNES specifically is real-time-feasible
//! on the real board is unverified and unverifiable from this desktop
//! sim; each was implemented here because it was asked for, with that
//! caveat intact, not because the underlying concern was resolved.
//!
//! **The actual cores**: [`tetanes_core`](https://docs.rs/tetanes-core)
//! for NES, [`super_sabicom`](https://docs.rs/super-sabicom) for SNES,
//! `phosphor-core`/`phosphor-machines` for Arcade, and
//! [`rboy`](https://github.com/mvdnes/rboy) for Game Boy/Game Boy Color
//! -- all real, vendored (the arcade and Game Boy cores via git
//! dependency, since neither is on crates.io) rather than hand-written
//! from scratch, the same "borrow a real, proven implementation instead
//! of reinventing complex DSP/emulation" call this project already
//! makes for `vendor/eurorack`'s Mutable Instruments code. All pure
//! Rust today (this crate already requires `std`, so that's not a
//! regression), though a real firmware port would still need to
//! confirm each (or a `no_std` equivalent) fits the N6's actual RAM
//! budget -- not something verifiable from a desktop sim. Of these
//! four, Game Boy has the *best* real-hardware precedent: genuine
//! GB-class emulation already runs on MCU-class chips (ESP32 and
//! similar), meaningfully more encouraging than the SNES/arcade
//! caveats above.
//!
//! **Neo Geo is a fifth, different case**: no complete, vendorable
//! system emulator exists for it anywhere in the Rust ecosystem (see
//! `neogeo_core.rs`'s own module doc comment for what was actually
//! checked). Its 68000/Z80 CPUs and its YM2610 sound chip are real,
//! vendored crates, same as the other four's cores -- but the memory
//! map and video are hand-written against public hardware
//! documentation instead of borrowed, a deliberate, flagged exception
//! to this file's usual "vendor a real implementation" rule. Video
//! (fix/text layer and sprites), sound, VBlank, and Metal Slug 3's own
//! NEO-SMA and CMC42 protection chips (real 68000 program decryption/
//! bankswitching and real sprite/fix-layer graphics decryption, both
//! reimplemented from MAME's own source) all work; most other real
//! cartridges' own protection chips aren't implemented at all yet.
//!
//! **What's real here**: ROM scanning per console (ROMs are
//! copyrighted and must stay user-supplied -- see `.gitignore`'s
//! `roms/`; for Arcade this means a directory of individually-named
//! chip dumps per game, matching a real MAME romset's own file names --
//! see `scan_arcade_roms`), loading and running an actual game via
//! whichever core, real video (each core's own rendered frame,
//! converted to RGBA and blitted/scaled into the device's own
//! `FrameBuffer`) and real audio (game audio pushed through `AudioBus`
//! exactly like every other app's output -- so Beads/Black Hole/Tape/
//! the mixer can process live game audio, per the original ask), real
//! controller input (the 4x4 grid mapped to a standard pad per console,
//! generically by control *kind* for Arcade -- see
//! `apply_arcade_pad_input` -- since arcade control schemes vary by
//! game), a real PS5 DualSense (see gamepad.rs), and real save states
//! (each core's own serialize/deserialize, to per-ROM files outside the
//! repo).
//!
//! **What's deliberately not attempted yet** (matches the spec's own
//! "missing for full products" list): cover art/metadata, a favorites
//! list, player 2 and analog controls (trackball/spinner/wheel -- see
//! `apply_arcade_pad_input`'s own doc comment), and any in-game overlay
//! beyond the plain menu below (no long-press gesture exists in `Input`
//! to hang one off of). F3 (this OS's real Start/Stop button, see
//! `App::running`) already does the one overlay action that matters
//! most for a game: Pause/Resume.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::apps::neogeo_core::NeoGeoMachine;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use crate::util::{accelerate, AtomicF32};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use meru_interface::EmulatorCore;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tetanes_core::control_deck::ControlDeck;
use tetanes_core::input::{JoypadBtn, Player};

/// The NTSC family's real frame period -- 1 / 60.0988 Hz, not a
/// rounded 1/60s. Both consoles here happen to share it (SNES NTSC is
/// the same rate). See `tick`'s real-time pacing loop for why this
/// matters.
const FRAME_SECONDS: f32 = 1.0 / 60.0988;
const NUM_SAVE_SLOTS: usize = 4;
/// Capped so a paused/backgrounded game (nothing draining the audio
/// ring buffer) can't grow it without bound -- see
/// `RetroAudioProcessor::process`.
const MAX_QUEUED_AUDIO_SAMPLES: usize = 96_000; // interleaved stereo pairs, so ~1s at 48kHz

const NES_ROMS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/nes");
const NES_SAVES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/saves/nes");
const NES_EXTENSIONS: [&str; 1] = ["nes"];

const SNES_ROMS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/snes");
const SNES_SAVES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/saves/snes");
// Matches `super_sabicom::CORE_INFO.file_extensions` exactly -- the
// real distributed formats for this console (sfc/smc are the common
// ones; swc/fig are older copier-specific headers the core also
// accepts).
const SNES_EXTENSIONS: [&str; 4] = ["sfc", "smc", "swc", "fig"];

const VIDEO_SCALE_NAMES: [&str; 2] = ["Fit", "Stretch"];

const ARCADE_ROMS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/arcade");
const ARCADE_SAVES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/saves/arcade");

const GB_ROMS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/gb");
const GB_SAVES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/saves/gb");
const GB_EXTENSIONS: [&str; 2] = ["gb", "gbc"];
/// The real Game Boy's own refresh rate -- distinct enough from the
/// NES/SNES family's 60.0988Hz (and from arcade boards' own per-game
/// rates) to matter for the same real-time-pacing reason those already
/// get their own rate.
const GB_FRAME_SECONDS: f32 = 1.0 / 59.7275;

const NEOGEO_ROMS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/neogeo");
const NEOGEO_SAVES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/saves/neogeo");
/// Where a real, user-supplied SNK BIOS dump goes, if the user has one
/// -- shared system firmware, not part of any individual cartridge
/// folder, so it lives in its own subdirectory of `NEOGEO_ROMS_DIR`
/// rather than needing to be copied into every cartridge's own folder.
/// `scan_neogeo_roms` only lists folders containing a `p1.rom`-suffixed
/// file, so a BIOS-only folder here never shows up as a fake
/// "cartridge". See `neogeo_core.rs`'s own module doc comment for why
/// none is vendored.
const NEOGEO_BIOS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/neogeo/bios");
/// The real Neo Geo's own hardware-derived refresh rate: a 24MHz clock
/// / 4 = 6MHz pixel clock / 384 pixels-per-line = 15.625kHz horizontal
/// rate / 264 lines -- see
/// <https://wiki.neogeodev.org/index.php?title=Framerate>. Distinct
/// from every other console/arcade rate already in this file.
const NEOGEO_FRAME_SECONDS: f32 = 1.0 / 59.1856;

// --- Retro's own palette: a CRT-adjacent dark charcoal/green, distinct
// from every DSP app's own identity since this screen is mostly a game
// video surface, not a synth panel. ---

const RETRO_BG: Rgb565 = Rgb565::new(1, 3, 2);
const RETRO_TITLE: Rgb565 = Rgb565::new(24, 55, 19);
const RETRO_ACCENT: Rgb565 = Rgb565::new(6, 45, 12);
const RETRO_DIM: Rgb565 = Rgb565::new(10, 20, 9);
const RETRO_CHIP_BG: Rgb565 = RETRO_ACCENT;

/// Which system is currently selected -- the "Console" menu row this
/// whole refactor is for, so switching doesn't mean diving into a
/// submenu. Only affects which ROM library `Rom` browses and which
/// core `load_selected_rom` builds; a game already running keeps
/// running on whichever core loaded it regardless of what `Console`
/// is set to afterward.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Console {
    Nes,
    Snes,
    /// Not "MAME" itself (see the module doc comment for why real MAME
    /// has no realistic path here) -- a fixed set of individually
    /// implemented classic boards from `phosphor-machines`. Its own
    /// registry (see `scan_arcade_roms`), not a fixed extension list,
    /// decides what shows up: arcade "ROMs" are a whole directory of
    /// individually named chip dumps per game, not one self-contained
    /// file the way a console cartridge is.
    Arcade,
    /// Game Boy / Game Boy Color -- see `rboy`'s own doc comment
    /// (Cargo.toml) for why this is the best-precedented of the four
    /// for real STM32N6 feasibility.
    Gb,
    /// Neo Geo -- unlike the other four, no complete vendorable system
    /// emulator exists to borrow (see `neogeo_core.rs`'s own module
    /// doc comment); the 68000/Z80 CPU cores and the YM2610 sound chip
    /// are real, vendored implementations, but the memory map and
    /// video are hand-written against public hardware documentation.
    /// Metal Slug 3's real NEO-SMA protection (68000 decryption and
    /// bankswitching) and its separate CMC42 graphics-decryption chip
    /// (sprite/fix-layer C-ROM data) are both implemented, reimplemented
    /// from MAME's own source -- but `detect_neogeo_protection` only
    /// applies them to a P2 ROM that's actually the real, full-size
    /// (8MB) encrypted layout; a smaller P2 is treated as an
    /// already-decrypted dump (confirmed against this repo's own real
    /// cartridge folder: its file timestamps predate MAME's ~2008
    /// public disclosure of NEO-SMA, and its raw, only-word-swapped P1
    /// already carries the documented real SSP ($0010F300) and a
    /// legitimate entry-point jump, with no decryption needed at all).
    ///
    /// Sprite auto-animation is real. CMC42's exact left/right sprite
    /// orientation is still flagged unverified in `neogeo_core.rs`.
    /// Most other real cartridges' own protection chips aren't
    /// implemented at all -- those will surface the same "emulation
    /// error, reload the ROM" this app already shows for any other
    /// core's crash.
    ///
    /// **A real BIOS now loads if the user supplies one** (see
    /// `NEOGEO_BIOS_DIR`, `load_neogeo_bios`, and `NeoGeoBus`'s own doc
    /// comment) -- a real cartridge's own code makes a legitimate call
    /// into $C00000+ (the fixed BIOS ROM window every cartridge relies
    /// on for standard system services), which previously read back
    /// zeroed memory and ran off into it as bogus "code" with no BIOS
    /// mapped there. With a real BIOS supplied, that call now runs real
    /// code and returns properly, and a real, running Metal Slug 3
    /// session does start writing real tilemap data into VRAM (confirmed:
    /// 2336 real, non-zero VRAM words within the first ~100 frames).
    ///
    /// **Still no visible video, though**: after that initial burst,
    /// execution parks for a very long time (confirmed sustained across
    /// 1000+ real emulated frames, i.e. tens of seconds of in-game time)
    /// in a tight busy-wait inside the BIOS itself (`TST.B $10FE8C` /
    /// `BNE`), polling a plain work-RAM byte. Several real, concrete
    /// leads were checked and ruled out rather than guessed away:
    /// - VBlank interrupts are confirmed still firing and reaching the
    ///   cartridge's real vector-25 handler ($002654) throughout --
    ///   not a broken interrupt path.
    /// - The Z80 sound CPU is confirmed alive and doing real work: it
    ///   receives a real command (0x03) via NMI, executes a large,
    ///   varied real initialization sequence (confirmed via PC tracing:
    ///   dozens of distinct real addresses, including a real per-entry
    ///   loop over what looks like a channel/voice table), and settles
    ///   into its own real idle loop -- not crashed or stuck immediately.
    /// - The real watchdog-kick address ($300001, per MAME's own
    ///   `neogeo.cpp` -- a real hardware watchdog resets the system
    ///   after ~0.13s unless kicked, and MAME's own source notes some
    ///   games deliberately let it expire once to reinitialize backup
    ///   RAM) is being written continuously (hundreds of thousands of
    ///   times over the run) -- ruling out "deliberately waiting to be
    ///   reset": a cartridge that wanted that would stop kicking, not
    ///   keep servicing it.
    /// - The polled byte was confirmed to genuinely read 0 at least once
    ///   very early (frame 0), so it isn't hardwired to a stuck value at
    ///   the storage level -- something legitimately clears it early on
    ///   but then never again for a long, sustained stretch.
    ///
    /// None of this pins down what real, not-yet-modeled piece of
    /// hardware or timing is supposed to clear that byte on an ongoing
    /// basis. Further progress here would benefit from a real reference
    /// trace (e.g. running this exact ROM in MAME with its debugger) to
    /// see what actually clears it on real hardware -- something this
    /// session doesn't have access to.
    NeoGeo,
}
const CONSOLE_NAMES: [&str; 5] = ["NES", "SNES", "Arcade", "Game Boy", "Neo Geo"];
const CONSOLES: [Console; 5] = [Console::Nes, Console::Snes, Console::Arcade, Console::Gb, Console::NeoGeo];

impl Console {
    fn name(self) -> &'static str {
        CONSOLE_NAMES[CONSOLES.iter().position(|&c| c == self).unwrap()]
    }
    fn roms_dir(self) -> &'static str {
        match self {
            Console::Nes => NES_ROMS_DIR,
            Console::Snes => SNES_ROMS_DIR,
            Console::Arcade => ARCADE_ROMS_DIR,
            Console::Gb => GB_ROMS_DIR,
            Console::NeoGeo => NEOGEO_ROMS_DIR,
        }
    }
    fn saves_dir(self) -> &'static str {
        match self {
            Console::Nes => NES_SAVES_DIR,
            Console::Snes => SNES_SAVES_DIR,
            Console::Arcade => ARCADE_SAVES_DIR,
            Console::Gb => GB_SAVES_DIR,
            Console::NeoGeo => NEOGEO_SAVES_DIR,
        }
    }
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Console::Nes => &NES_EXTENSIONS,
            Console::Snes => &SNES_EXTENSIONS,
            Console::Gb => &GB_EXTENSIONS,
            // Unused -- `rescan_roms` takes the registry-driven
            // `scan_arcade_roms` path for this console instead of
            // `scan_roms`/extension matching.
            Console::Arcade => &[],
            // Unused -- `rescan_roms` takes the directory-scanning
            // `scan_neogeo_roms` path instead, the same reason Arcade
            // does (a cart is a folder of individually named chip
            // dumps, not one self-contained file).
            Console::NeoGeo => &[],
        }
    }
}

/// Either core, loaded and running -- see the module doc comment for
/// why this is a `match`-selected enum of statically-linked cores
/// rather than a dynamically loaded one.
enum Deck {
    Nes(Box<ControlDeck>),
    Snes(Box<super_sabicom::Snes>),
    Arcade(Box<dyn phosphor_core::core::machine::FrontendMachine>),
    /// The second element is *this app's own* handle onto the audio
    /// `rboy::Device::enable_audio` was given (see `GbAudioSink`) --
    /// `rboy`'s audio path is push-based (a callback trait), unlike
    /// the other three cores' pull-based `audio_samples()`/
    /// `audio_buffer()`/`fill_audio()`, so this is what lets the app
    /// drain what the callback already pushed after each `do_cycle`
    /// burst.
    Gb(Box<rboy::device::Device>, Arc<Mutex<(Vec<f32>, Vec<f32>)>>),
    /// See `neogeo_core`'s own module doc comment for exactly what's
    /// real here (68000/Z80 CPUs, memory map, fix-layer video) and
    /// what isn't yet (sprites, sound, most cartridges' protection
    /// chips).
    NeoGeo(Box<NeoGeoMachine>),
}

/// Real SNES hardware's APU/S-DSP always renders audio at a fixed
/// 32000Hz -- confirmed directly off `super_sabicom::Snes::
/// audio_buffer().sample_rate` at runtime, not assumed -- and
/// `super_sabicom` has no `set_sample_rate`-style knob to resample
/// that internally the way `tetanes_core::ControlDeck` does for NES.
/// Queueing those samples straight into `AudioBridge` at 32000Hz while
/// the real audio thread drains the queue at the device's actual rate
/// (48000Hz here, a 1.5x mismatch) starves it roughly a third of the
/// time -- periodic silence gaps, i.e. exactly "glitchy sound" with a
/// perfectly fine frame rate, since video isn't rate-mismatched at
/// all. This is a small persistent linear resampler (state carried
/// across calls, same idea `sample_drum.rs`'s own linear-interpolation
/// playback already uses) that turns SNES's fixed 32000Hz stream into
/// the real device rate before it ever reaches the queue.
#[derive(Default)]
struct LinearResampler {
    /// Fractional read position into `pending`, in input frames (not
    /// samples -- one frame = one L+R pair here).
    pos: f32,
    /// Interleaved input samples not yet fully consumed, carried over
    /// from the previous call so resampling stays continuous across
    /// per-emulated-frame audio chunks instead of restarting (and
    /// clicking) at 0 every time.
    pending: Vec<f32>,
}

impl LinearResampler {
    /// `input`/output are both interleaved `channels`-wide frames.
    fn resample(&mut self, input: &[f32], in_rate: f32, out_rate: f32, channels: usize) -> Vec<f32> {
        if in_rate <= 0.0 || out_rate <= 0.0 || channels == 0 {
            return Vec::new();
        }
        self.pending.extend_from_slice(input);
        let in_frames = self.pending.len() / channels;
        if in_frames < 2 {
            return Vec::new();
        }
        let ratio = in_rate / out_rate;
        let mut out = Vec::new();
        while self.pos + 1.0 < in_frames as f32 {
            let i0 = self.pos as usize;
            let frac = self.pos - i0 as f32;
            for c in 0..channels {
                let a = self.pending[i0 * channels + c];
                let b = self.pending[(i0 + 1) * channels + c];
                out.push(a + (b - a) * frac);
            }
            self.pos += ratio;
        }
        let consumed_frames = (self.pos as usize).min(in_frames.saturating_sub(1));
        self.pending.drain(0..consumed_frames * channels);
        self.pos -= consumed_frames as f32;
        out
    }
}

struct RomEntry {
    name: String,
    path: PathBuf,
    /// Set only for `Console::Arcade` -- which registered
    /// `phosphor_machines` driver this directory's ROMs belong to (see
    /// `scan_arcade_roms`). NES/SNES entries load directly from `path`
    /// (one self-contained file); an Arcade entry loads via this
    /// machine's own `create` factory against a `RomSet` built from
    /// the whole directory instead.
    machine: Option<&'static phosphor_machines::registry::MachineEntry>,
}

/// Recursively finds every file under `dir` matching one of
/// `extensions` (case-insensitive), sorted for a stable browse order --
/// same convention `sample_drum.rs`'s own `scan_samples` uses for its
/// (also user-supplied, also outside the repo) library.
fn scan_roms(dir: &Path, extensions: &[&str]) -> Vec<RomEntry> {
    let mut out = Vec::new();
    scan_dir(dir, extensions, &mut out);
    out
}

fn scan_dir(dir: &Path, extensions: &[&str], out: &mut Vec<RomEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            scan_dir(&path, extensions, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.iter().any(|ext| e.eq_ignore_ascii_case(ext))) {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
            out.push(RomEntry { name, path, machine: None });
        }
    }
}

/// Finds every registered `phosphor_machines` driver that has a
/// matching ROM directory under `dir` -- a MAME-format romset (e.g.
/// `pacman.zip`) extracted into a folder of the same name (see
/// `MachineEntry::rom_names`, which lists every valid name MAME itself
/// accepts for that game, in priority order). Registry-driven rather
/// than a hardcoded list, so a new machine `phosphor_machines` adds
/// upstream shows up here automatically the next time this crate is
/// updated, with zero changes to this app.
fn scan_arcade_roms(dir: &Path) -> Vec<RomEntry> {
    let mut out = Vec::new();
    for entry in phosphor_machines::registry::all() {
        for &rom_name in entry.rom_names {
            let candidate = dir.join(rom_name);
            if candidate.is_dir() {
                out.push(RomEntry { name: entry.name.to_string(), path: candidate, machine: Some(entry) });
                break;
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Finds every subdirectory of `dir` that looks like a real Neo Geo
/// cartridge dump -- containing at least one file whose name ends in
/// `p1.rom` (case-insensitive; real dumps are conventionally named
/// like `256-p1.rom`, prefixed by the game's own MAME set number, so
/// matching the suffix rather than an exact name works across sets).
/// No registry exists for this the way `phosphor_machines` provides
/// for Arcade -- Neo Geo doesn't have an equivalent open catalog of
/// every cart's exact file layout -- so the folder's own name becomes
/// the display name directly, same as NES/SNES/GB's plain filename
/// convention.
fn scan_neogeo_roms(dir: &Path) -> Vec<RomEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&path) else { continue };
        let has_p1 = files.flatten().any(|f| f.file_name().to_str().is_some_and(|n| n.to_ascii_lowercase().ends_with("p1.rom")));
        if has_p1 {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
            out.push(RomEntry { name, path, machine: None });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Loads the specific ROM files `neogeo_core::NeoGeoMachine::new`
/// needs out of a cartridge folder `scan_neogeo_roms` already
/// confirmed contains at least a P1 ROM -- matching by filename suffix
/// (`p1.rom`, `p2.rom`, `m1.rom`, `s1.rom`, `c1.rom` through `c8.rom`,
/// and `v1.rom` through `v4.rom`, case-insensitive) rather than
/// requiring an exact naming convention, since the numeric prefix
/// varies by cart. Missing files simply come back empty --
/// `NeoGeoMachine::new` and `neogeo_core`'s decode functions already
/// treat an empty ROM as "no data for this yet" rather than panicking.
fn load_neogeo_cartridge(dir: &Path) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<Vec<u8>>, Vec<u8>) {
    let find = |suffix: &str| -> Vec<u8> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .find(|f| f.file_name().to_str().is_some_and(|n| n.to_ascii_lowercase().ends_with(suffix)))
            .and_then(|f| std::fs::read(f.path()).ok())
            .unwrap_or_default()
    };
    let p1 = find("p1.rom");
    let p2 = find("p2.rom");
    let m1 = find("m1.rom");
    let s1 = find("s1.rom");
    let c_roms = (1..=8).map(|n| find(&format!("c{n}.rom"))).collect();
    // V-ROMs (ADPCM sample data) concatenate in order, same convention
    // as the C-ROMs' odd/even split in `concat_c_roms`.
    let v_rom = (1..=4).flat_map(|n| find(&format!("v{n}.rom"))).collect();
    (p1, p2, m1, s1, c_roms, v_rom)
}

/// A real, user-supplied SNK BIOS dump from `NEOGEO_BIOS_DIR`, if one
/// is present -- any file in there, whichever regional revision the
/// user dropped in (see `NEOGEO_BIOS_DIR`'s own doc comment). Empty
/// (no BIOS mapped) if the folder doesn't exist or has no files.
fn load_neogeo_bios() -> Vec<u8> {
    std::fs::read_dir(NEOGEO_BIOS_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|entry| std::fs::read(entry.path()).ok())
        .unwrap_or_default()
}

/// Detects a Neo Geo cartridge's real protection chip -- no per-game
/// registry exists (see `scan_neogeo_roms`'s own doc comment), so this
/// combines the folder-name convention already used there with a real
/// structural check on the P2 ROM's own size. Only Metal Slug 3/3A's
/// NEO-SMA is implemented so far (see `neogeo_core::Protection`'s own
/// doc comment).
///
/// The name alone isn't enough: MAME's real, still-encrypted `mslug3`
/// romset splits its banked program data across two real 4MB chips
/// (`256-pg1.p1` + `256-pg2.p2`, 8MB combined -- confirmed directly
/// from MAME's own `ROM_START(mslug3)`) plus a real, separately-dumped
/// 256KB ROM embedded in the SMA chip itself (`green.neo-sma`) that
/// supplies part of the fixed bank; encrypted carts also have no S1
/// dump at all (an MVS board without SMA has none -- the fix layer's
/// graphics come from the C-ROMs instead). A folder named
/// "metalslug3"/"mslug3" but shaped like a single, already-decrypted
/// 4MB P2 with its own standalone S1 file (confirmed against a real,
/// legally-owned dump whose C/M/P/S/V file timestamps are all
/// 2001-2003 -- years before MAME's ~2008 public disclosure of the
/// NEO-SMA algorithm) is a pre-decrypted release, not a raw MAME dump:
/// running this project's own from-scratch SMA/CMC42 decryption
/// against data that's already been decrypted once just re-scrambles
/// it into garbage (confirmed: the CPU ran for 30,000,000 real
/// instructions afterward without a single VRAM or palette write).
/// Only apply SMA when the P2 data is actually large enough to be the
/// real encrypted layout.
fn detect_neogeo_protection(rom_name: &str, p2_len: usize) -> crate::apps::neogeo_core::Protection {
    let name_lower = rom_name.to_ascii_lowercase();
    let name_matches = name_lower.contains("mslug3") || name_lower.contains("metal slug 3") || name_lower.contains("metalslug3");
    if name_matches && p2_len >= 0x800000 {
        crate::apps::neogeo_core::Protection::SmaMslug3
    } else {
        crate::apps::neogeo_core::Protection::None
    }
}

fn save_state_path(console: Console, rom_name: &str, slot: usize) -> PathBuf {
    // ROM names, not indices -- same "still correct if the library
    // changes" reasoning `sample_drum.rs`'s presets use, since a save
    // state tied to a raw list index would silently load onto the
    // wrong game the moment the ROM folder's contents change.
    Path::new(console.saves_dir()).join(format!("{rom_name}.slot{slot}.state"))
}

/// One nearest-neighbor-scaled, letterboxed blit of a console's real
/// rendered RGBA output into the device's own framebuffer -- integer-
/// scaled ("Fit") preserves the original pixel aspect exactly (no
/// smeared/uneven pixels), "Stretch" fills the whole plot area
/// instead. Real per-pixel work, not a placeholder: every device
/// pixel maps back to exactly the source pixel it should show.
/// `src_w`/`src_h` are read from the core's own frame buffer each
/// frame (not a fixed constant) since consoles -- and even individual
/// games on the same console -- can render at different resolutions.
fn blit_frame(fb: &mut FrameBuffer, rgba: &[u8], src_w: usize, src_h: usize, x: i32, y: i32, w: i32, h: i32, stretch: bool) {
    if src_w == 0 || src_h == 0 || rgba.len() < src_w * src_h * 4 {
        return;
    }
    let (scale_x, scale_y) = if stretch {
        (w as f32 / src_w as f32, h as f32 / src_h as f32)
    } else {
        let scale = (w as f32 / src_w as f32).min(h as f32 / src_h as f32);
        (scale, scale)
    };
    let out_w = (src_w as f32 * scale_x).round() as i32;
    let out_h = (src_h as f32 * scale_y).round() as i32;
    let off_x = x + (w - out_w) / 2;
    let off_y = y + (h - out_h) / 2;

    for row in 0..out_h {
        let src_y = ((row as f32 / scale_y) as usize).min(src_h - 1);
        for col in 0..out_w {
            let src_x = ((col as f32 / scale_x) as usize).min(src_w - 1);
            let idx = (src_y * src_w + src_x) * 4;
            let (r, g, b) = (rgba[idx], rgba[idx + 1], rgba[idx + 2]);
            let color = Rgb565::new(r >> 3, g >> 2, b >> 3);
            fb.draw_iter(core::iter::once(embedded_graphics::Pixel(Point::new(off_x + col, off_y + row), color))).ok();
        }
    }
}

/// `super_sabicom`'s frame buffer is `Color { r, g, b }` pixels, not
/// packed RGBA bytes -- converts once per frame into the same RGBA8
/// shape `blit_frame`/the Slint preview both already expect, so
/// neither has to know which core produced the pixels.
fn snes_frame_to_rgba(fb: &meru_interface::FrameBuffer) -> (usize, usize, Vec<u8>) {
    let mut rgba = Vec::with_capacity(fb.buffer.len() * 4);
    for c in &fb.buffer {
        rgba.extend_from_slice(&[c.r, c.g, c.b, 255]);
    }
    (fb.width, fb.height, rgba)
}

/// Both `phosphor_machines` and `rboy` render into a flat RGB24 buffer
/// (3 bytes/pixel, no alpha) rather than returning their own owned
/// pixel type -- converts into the same RGBA8 shape `blit_frame`/the
/// Slint preview both already expect.
fn rgb24_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
    for px in rgb.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    rgba
}

fn arcade_frame_to_rgba(machine: &dyn phosphor_core::core::machine::FrontendMachine) -> (usize, usize, Vec<u8>) {
    let (w, h) = machine.display_size();
    let mut rgb = vec![0u8; w as usize * h as usize * 3];
    machine.render_frame(&mut rgb);
    (w as usize, h as usize, rgb24_to_rgba(&rgb))
}

/// `rboy::device::Device::enable_audio` takes ownership of a `Box<dyn
/// AudioPlayer>`, calling `play()` from inside `do_cycle` whenever its
/// internal `blip_buf` mixer has a batch ready -- push-based, unlike
/// the other three cores' pull-based audio methods. This is the other
/// half of that callback: it just appends into a buffer this app kept
/// its own `Arc<Mutex<..>>` handle to (see `Deck::Gb`), so `tick()` can
/// drain what got pushed after each `do_cycle` burst, same as it pulls
/// from the other cores' own buffers.
struct GbAudioSink {
    buffer: Arc<Mutex<(Vec<f32>, Vec<f32>)>>,
    rate: u32,
}

impl rboy::AudioPlayer for GbAudioSink {
    fn play(&mut self, left_channel: &[f32], right_channel: &[f32]) {
        let mut buf = self.buffer.lock().unwrap();
        buf.0.extend_from_slice(left_channel);
        buf.1.extend_from_slice(right_channel);
    }
    fn samples_rate(&self) -> u32 {
        self.rate
    }
    fn underflowed(&self) -> bool {
        // Always "yes, give me fresh samples" -- this app has no
        // double-buffering of its own to protect; `tick()`'s own
        // `MAX_QUEUED_AUDIO_SAMPLES` cap already bounds the shared
        // ring buffer downstream.
        true
    }
}

/// A best-effort readable message out of a `catch_unwind` payload --
/// `panic!("{x}")`/`.expect("...")`-style panics carry a `&str` or
/// `String`, which covers the vast majority in practice (including
/// `super_sabicom`'s own arithmetic-overflow panics, which come from
/// the compiler's built-in overflow checks, not a custom message).
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "core panicked (no message)".into())
}

/// Generic pad mapping for *any* registered `phosphor_machines` board --
/// unlike NES/SNES, arcade control schemes vary per game (a joystick
/// with 1 button, 2, 4, an 8-way vs 4-way stick, Coin/Start being
/// mandatory), so this reads each machine's own declared
/// `input_controls()` by *kind* (a real, typed enum -- `DigitalDirection`/
/// `Coin`/`Start`/`Action`/`Button` -- not a guess from its display
/// name) rather than hardcoding one game's layout. Coin/Start get fixed
/// pads (mandatory on every real cabinet: without a credit and a start
/// press, no game lets you move); the D-Pad gets its usual 0-3; every
/// other button-like control claims the next free pad in declaration
/// order. Player 2 and analog controls (trackball/spinner/wheel) are
/// deliberately out of scope for this first pass -- see the module doc
/// comment's own "not attempted yet" list.
fn apply_arcade_pad_input(machine: &mut dyn phosphor_core::core::machine::FrontendMachine, input: &Input) {
    use phosphor_core::core::machine::{Direction, InputEvent, InputKind};
    let mut next_button_pad = 4usize;
    for control in machine.input_controls() {
        if matches!(control.player, Some(player) if player != 1) {
            continue;
        }
        let pad = match control.kind {
            InputKind::DigitalDirection { direction: Direction::Up } => Some(0),
            InputKind::DigitalDirection { direction: Direction::Down } => Some(1),
            InputKind::DigitalDirection { direction: Direction::Left } => Some(2),
            InputKind::DigitalDirection { direction: Direction::Right } => Some(3),
            InputKind::Coin => Some(14),
            InputKind::Start => Some(15),
            InputKind::Action(_) | InputKind::Button => {
                if next_button_pad <= 13 {
                    let assigned = next_button_pad;
                    next_button_pad += 1;
                    Some(assigned)
                } else {
                    None // more buttons than this sim's 16 pads can cover -- extremely rare, drops the extras rather than panicking
                }
            }
            InputKind::Service | InputKind::AnalogAxis { .. } => None,
        };
        if let Some(pad) = pad {
            machine.handle_input(InputEvent::Button { id: control.id, pressed: input.grid[pad] });
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Console,
    Rom,
    SaveSlot,
    SaveState,
    LoadState,
    Reset,
    VideoScale,
    Volume,
}
const ROWS: [Selection; 8] = [
    Selection::Console,
    Selection::Rom,
    Selection::SaveSlot,
    Selection::SaveState,
    Selection::LoadState,
    Selection::Reset,
    Selection::VideoScale,
    Selection::Volume,
];

/// The one thing the audio-thread `RetroAudioProcessor` and the main-
/// thread `RetroApp` both touch -- everything else about the emulator
/// (the `Deck` itself, its video output, joypad state) lives only on
/// the main thread, since `tick()`/`draw()` already run there single-
/// threaded, same as every other app. Only the rendered audio has to
/// cross to the real-time audio callback thread. Always interleaved
/// stereo pairs (L, R, L, R, ...) regardless of which core produced
/// them -- NES's own mono output is just duplicated into both channels
/// when queued, so the consumer never needs to know which console is
/// playing.
struct AudioBridge {
    queue: Mutex<VecDeque<f32>>,
    /// The real device sample rate, discovered from the audio
    /// thread's own `process()` calls (the only place it's known) and
    /// read back by `tick()` so the deck's own sample rate can track
    /// it -- see `RetroApp::sync_sample_rate`.
    sample_rate: AtomicF32,
}

pub struct RetroApp {
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    console: Console,
    roms: Vec<RomEntry>,
    rom_index: AtomicUsize,
    save_slot: AtomicUsize,
    video_scale: AtomicUsize,
    volume: AtomicF32,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    bridge: Arc<AudioBridge>,
    deck: Option<Deck>,
    loaded_rom_name: Option<String>,
    running: bool,
    configured_sample_rate: f32,
    /// The active console's own last rendered frame, already converted
    /// to RGBA and ready to blit -- `(width, height, rgba_bytes)`, not
    /// a fixed-size array, since different consoles (and even
    /// different games on the same console) can render at different
    /// resolutions. Kept as RGBA (not yet downsampled to Rgb565) since
    /// `blit_frame` needs to scale before quantizing colors, not after
    /// (quantizing first would bake in nearest-neighbor duplication
    /// artifacts).
    last_frame: Option<(usize, usize, Vec<u8>)>,
    status: String,
    /// Real wall-clock pacing for stepping the emulator -- see `tick`'s
    /// doc comment for why this exists (a naive "one frame per `tick`"
    /// starves the audio ring buffer whenever the host UI loop runs
    /// slower than the console's own ~60.0988Hz refresh rate, which is
    /// exactly what "glitchy"/crackling audio was).
    last_tick: Instant,
    frame_accum: f32,
    /// Whether the menu/ROM list is showing (`true`) or the game is
    /// full-screen (`false`) -- toggled by `knob1_press`, which this
    /// app otherwise leaves unused (see `tick`). Starts `true` so
    /// there's something to select a ROM from; flips to full-screen
    /// automatically the moment a game actually loads.
    menu_visible: bool,
    /// SNES/Arcade only -- see `LinearResampler`'s own doc comment for
    /// why NES doesn't need one (it resamples internally). Shared by
    /// both since `self.deck` is only ever one variant at a time; reset
    /// whenever a new ROM loads (see `load_selected_rom`) so stale
    /// carried-over samples from a previous game never bleed into a
    /// fresh one.
    resampler: LinearResampler,
    /// `rboy::device::Device::keydown`/`keyup` are edge-triggered
    /// calls, unlike the other three cores' level-state `set_button`/
    /// `set_input` -- this is what `apply_pad_input`'s Game Boy branch
    /// diffs the live grid against to know which pads actually changed
    /// since last tick. Reset whenever a new ROM loads so a pad held
    /// through a game switch doesn't read as a fresh press it never
    /// was.
    gb_prev_grid: [bool; 16],
}

impl RetroApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Retro", &modbus);
        let console = Console::Nes;
        Self {
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            console,
            roms: scan_roms(Path::new(console.roms_dir()), console.extensions()),
            rom_index: AtomicUsize::new(0),
            save_slot: AtomicUsize::new(0),
            video_scale: AtomicUsize::new(0),
            volume: AtomicF32::new(0.8),
            mix_level,
            ext_mix_level,
            bus_out: audio_bus.register("Retro"),
            bridge: Arc::new(AudioBridge { queue: Mutex::new(VecDeque::new()), sample_rate: AtomicF32::new(48_000.0) }),
            deck: None,
            loaded_rom_name: None,
            running: false,
            // Matches `AudioBridge::sample_rate`'s own default -- a
            // ROM loaded before the first `tick()` (e.g. driven
            // directly, as a test might) must never configure the
            // deck for a bogus near-zero sample rate; `sync_sample_rate`
            // corrects this to the real device rate on the very next
            // tick either way.
            configured_sample_rate: 48_000.0,
            last_frame: None,
            status: Self::no_selection_status(console, &scan_roms(Path::new(console.roms_dir()), console.extensions())),
            last_tick: Instant::now(),
            frame_accum: 0.0,
            menu_visible: true,
            resampler: LinearResampler::default(),
            gb_prev_grid: [false; 16],
        }
    }

    fn no_selection_status(console: Console, roms: &[RomEntry]) -> String {
        if roms.is_empty() {
            format!("no {} ROMs found in {}", console.name(), console.roms_dir())
        } else {
            "select a ROM, hold SELECT to load".into()
        }
    }

    fn rom_name(&self) -> &str {
        self.roms.get(self.rom_index.load(Ordering::Relaxed)).map(|r| r.name.as_str()).unwrap_or("(none)")
    }

    /// Re-scans the *current* `console`'s ROM library, keeping the
    /// selection by name (not index -- same reasoning `sample_drum.rs`'s
    /// presets already use) if that ROM is still there. Shared by
    /// `on_enter` (picks up a file dropped in without restarting) and
    /// switching `Console` (an entirely different library to browse).
    fn rescan_roms(&mut self) {
        let previously_selected = self.roms.get(self.rom_index.load(Ordering::Relaxed)).map(|r| r.name.clone());
        self.roms = match self.console {
            Console::Arcade => scan_arcade_roms(Path::new(ARCADE_ROMS_DIR)),
            Console::NeoGeo => scan_neogeo_roms(Path::new(NEOGEO_ROMS_DIR)),
            _ => scan_roms(Path::new(self.console.roms_dir()), self.console.extensions()),
        };
        let new_index = previously_selected.as_deref().and_then(|name| self.roms.iter().position(|r| r.name == name)).unwrap_or(0);
        self.rom_index.store(new_index, Ordering::Relaxed);
        if self.loaded_rom_name.is_none() {
            self.status = Self::no_selection_status(self.console, &self.roms);
        }
    }

    fn load_selected_rom(&mut self) {
        let Some(rom) = self.roms.get(self.rom_index.load(Ordering::Relaxed)) else {
            self.status = "no ROM selected".into();
            return;
        };
        let rom_path = rom.path.clone();
        let rom_name = rom.name.clone();
        let machine_entry = rom.machine;
        let result: Result<Deck, String> = match self.console {
            Console::Nes => {
                let mut deck = ControlDeck::new();
                deck.set_sample_rate(self.configured_sample_rate.max(1.0));
                deck.load_rom_path(&rom_path).map(|_| Deck::Nes(Box::new(deck))).map_err(|e| e.to_string())
            }
            Console::Snes => std::fs::read(&rom_path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| super_sabicom::Snes::try_from_file(&bytes, None, &Default::default()).map_err(|e| e.to_string()))
                .map(|snes| Deck::Snes(Box::new(snes))),
            Console::Arcade => match machine_entry {
                Some(entry) => phosphor_machines::rom_loader::RomSet::from_directory(&rom_path)
                    .map_err(|e| e.to_string())
                    .and_then(|romset| (entry.create)(&romset).map_err(|e| e.to_string()))
                    .map(Deck::Arcade),
                None => Err("internal error: this ROM entry has no machine driver attached".into()),
            },
            Console::Gb => std::fs::read(&rom_path).map_err(|e| e.to_string()).and_then(|bytes| {
                // Always CGB mode -- a real Game Boy Color is backward
                // compatible with plain DMG carts and auto-detects
                // which one it's holding, so this always gets the best
                // available experience (color on GBC games, faithful
                // DMG behavior otherwise) rather than needing its own
                // menu row to pick.
                rboy::device::Device::new_cgb_from_buffer(bytes, false, None).map_err(|e| e.to_string()).map(|mut device| {
                    let sink_buf = Arc::new(Mutex::new((Vec::new(), Vec::new())));
                    device.enable_audio(Box::new(GbAudioSink { buffer: Arc::clone(&sink_buf), rate: self.configured_sample_rate.max(1.0) as u32 }), true);
                    Deck::Gb(Box::new(device), sink_buf)
                })
            }),
            Console::NeoGeo => {
                let (p1, p2, m1, s1, c_roms, v_rom) = load_neogeo_cartridge(&rom_path);
                if p1.is_empty() {
                    Err("no P1 ROM found in this cartridge folder".into())
                } else {
                    let protection = detect_neogeo_protection(&rom_name, p2.len());
                    Ok(Deck::NeoGeo(Box::new(NeoGeoMachine::new(p1, p2, m1, s1, c_roms, v_rom, protection, load_neogeo_bios()))))
                }
            }
        };
        match result {
            Ok(deck) => {
                self.loaded_rom_name = Some(rom_name.clone());
                self.deck = Some(deck);
                self.running = true;
                self.last_frame = None;
                self.frame_accum = 0.0;
                self.last_tick = Instant::now();
                self.bridge.queue.lock().unwrap().clear();
                self.resampler = LinearResampler::default();
                self.gb_prev_grid = [false; 16];
                self.status = format!("loaded {rom_name}");
                self.menu_visible = false;
            }
            Err(e) => {
                self.deck = None;
                self.loaded_rom_name = None;
                self.status = format!("couldn't load {rom_name}: {e}");
            }
        }
    }

    fn reset(&mut self) {
        if self.loaded_rom_name.is_some() {
            self.load_selected_rom();
        }
    }

    fn save_state(&mut self) {
        let (Some(deck), Some(rom_name)) = (self.deck.as_mut(), self.loaded_rom_name.clone()) else {
            self.status = "no game running to save".into();
            return;
        };
        let path = save_state_path(self.console, &rom_name, self.save_slot.load(Ordering::Relaxed));
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                self.status = format!("couldn't create {}: {e}", dir.display());
                return;
            }
        }
        let result = match deck {
            Deck::Nes(deck) => std::fs::File::create(&path).and_then(|file| deck.save_state(file).map_err(std::io::Error::other)).map_err(|e| e.to_string()),
            Deck::Snes(snes) => std::fs::write(&path, snes.save_state()).map_err(|e| e.to_string()),
            Deck::Arcade(machine) => {
                match machine.save_state() {
                    Some(bytes) => std::fs::write(&path, bytes).map_err(|e| e.to_string()),
                    None => Err("this game doesn't support save states".into()),
                }
            }
            // `rboy`'s public API has no on-demand full save-state
            // snapshot (its own save/load is a path-based, save-on-
            // Drop convention this app doesn't drive) -- battery-
            // backed SRAM is the real mechanism actual GB cartridges
            // use for persistent progress (Pokemon, Zelda, and every
            // other battery-save game), so that's what "Save"/"Load"
            // mean here: less granular than a true mid-scene snapshot,
            // but the historically honest equivalent, not a shortcut.
            Deck::Gb(device, _) => std::fs::write(&path, device.dumpram()).map_err(|e| e.to_string()),
            // No save-state format exists yet for this core -- see
            // `neogeo_core`'s own module doc comment for what's real
            // and what isn't.
            Deck::NeoGeo(_) => Err("save states aren't supported for Neo Geo yet".into()),
        };
        match result {
            Ok(()) => self.status = format!("saved slot {}", self.save_slot.load(Ordering::Relaxed) + 1),
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    fn load_state(&mut self) {
        let (Some(deck), Some(rom_name)) = (self.deck.as_mut(), self.loaded_rom_name.clone()) else {
            self.status = "no game running to load onto".into();
            return;
        };
        let path = save_state_path(self.console, &rom_name, self.save_slot.load(Ordering::Relaxed));
        let result: Result<(), String> = match deck {
            Deck::Nes(deck) => std::fs::File::open(&path).map_err(|e| e.to_string()).and_then(|file| deck.load_state(file).map_err(|e| e.to_string())),
            Deck::Snes(snes) => std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| snes.load_state(&bytes).map_err(|e| e.to_string())),
            Deck::Arcade(machine) => {
                std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| machine.load_state(&bytes).map_err(|e| e.to_string()))
            }
            Deck::Gb(device, _) => std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| device.loadram(&bytes).map_err(|e| e.to_string())),
            Deck::NeoGeo(_) => Err("save states aren't supported for Neo Geo yet".into()),
        };
        match result {
            Ok(()) => self.status = format!("loaded slot {}", self.save_slot.load(Ordering::Relaxed) + 1),
            Err(e) => self.status = format!("no save in slot {}: {e}", self.save_slot.load(Ordering::Relaxed) + 1),
        }
    }

    /// Applies the 4x4 grid's held state as a standard pad for
    /// whichever console is currently loaded, every tick before the
    /// emulator steps -- see the module doc comment for why there's no
    /// long-press overlay to steal any of these pads for menu use
    /// instead. NES only has 8 buttons (pads 8-15 unused); SNES uses
    /// all 12 (D-Pad + A/B/X/Y + L/R + Select/Start).
    fn apply_pad_input(&mut self, input: &Input) {
        let Some(deck) = self.deck.as_mut() else { return };
        match deck {
            Deck::Nes(deck) => {
                let pad = deck.joypad_mut(Player::One);
                const MAP: [(usize, JoypadBtn); 8] =
                    [(0, JoypadBtn::Up), (1, JoypadBtn::Down), (2, JoypadBtn::Left), (3, JoypadBtn::Right), (4, JoypadBtn::B), (5, JoypadBtn::A), (6, JoypadBtn::Select), (7, JoypadBtn::Start)];
                for (pad_index, button) in MAP {
                    pad.set_button(button, input.grid[pad_index]);
                }
            }
            Deck::Snes(snes) => {
                const MAP: [(usize, &str); 12] = [
                    (0, "Up"),
                    (1, "Down"),
                    (2, "Left"),
                    (3, "Right"),
                    (4, "B"),
                    (5, "A"),
                    (6, "Y"),
                    (7, "X"),
                    (8, "L"),
                    (9, "R"),
                    (10, "Select"),
                    (11, "Start"),
                ];
                let buttons: Vec<(String, bool)> = MAP.iter().map(|&(pad_index, name)| (name.to_string(), input.grid[pad_index])).collect();
                snes.set_input(&meru_interface::InputData { controllers: vec![buttons, vec![], vec![], vec![]] });
            }
            Deck::Arcade(machine) => apply_arcade_pad_input(machine.as_mut(), input),
            Deck::Gb(device, _) => {
                const MAP: [(usize, rboy::KeypadKey); 8] = [
                    (0, rboy::KeypadKey::Up),
                    (1, rboy::KeypadKey::Down),
                    (2, rboy::KeypadKey::Left),
                    (3, rboy::KeypadKey::Right),
                    (4, rboy::KeypadKey::B),
                    (5, rboy::KeypadKey::A),
                    (6, rboy::KeypadKey::Select),
                    (7, rboy::KeypadKey::Start),
                ];
                for (pad_index, key) in MAP {
                    let now = input.grid[pad_index];
                    if now != self.gb_prev_grid[pad_index] {
                        if now {
                            device.keydown(key);
                        } else {
                            device.keyup(key);
                        }
                    }
                }
                self.gb_prev_grid = input.grid;
            }
            // Real Neo Geo joypad (A/B/C/D + Start/Select), mapped onto
            // the grid following the same slot convention as the other
            // consoles above (0-3=D-Pad, 4-5=primary face buttons,
            // 6-7=Select/Start); pads 8-9 cover the Neo Geo's extra C/D
            // face buttons the other consoles don't have. See
            // `neogeo_core::NeoGeoBus::set_p1_input`'s own doc comment
            // for the real REG_P1CNT/REG_STATUS_B bit layouts this
            // feeds.
            Deck::NeoGeo(machine) => {
                let g = &input.grid;
                machine.bus.set_p1_input([g[0], g[1], g[2], g[3], g[5], g[4], g[8], g[9]], g[7], g[6]);
            }
        }
    }

    /// Reads whatever sample rate the audio thread has actually seen
    /// and re-configures the NES deck if it changed -- the deck itself
    /// only ever runs on this (main) thread, so this is a plain read/
    /// write, no lock needed beyond the bridge's own atomic.
    /// `super_sabicom` has no equivalent knob (its APU always renders
    /// at its own native rate); NES's `ControlDeck` does its own
    /// resampling internally, so this only ever needs to touch it.
    fn sync_sample_rate(&mut self) {
        let rate = self.bridge.sample_rate.get();
        if rate > 0.0 && (rate - self.configured_sample_rate).abs() > 0.5 {
            self.configured_sample_rate = rate;
            if let Some(Deck::Nes(deck)) = self.deck.as_mut() {
                deck.set_sample_rate(rate);
            }
        }
    }

    fn leaf_name(sel: Selection) -> &'static str {
        match sel {
            Selection::Console => "Console",
            Selection::Rom => "ROM",
            Selection::SaveSlot => "Save Slot",
            Selection::SaveState => "Save State",
            Selection::LoadState => "Load State",
            Selection::Reset => "Reset",
            Selection::VideoScale => "Video Scale",
            Selection::Volume => "Volume",
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Console => self.console.name().to_string(),
            Selection::Rom => self.rom_name().to_string(),
            Selection::SaveSlot => format!("{}", self.save_slot.load(Ordering::Relaxed) + 1),
            Selection::SaveState | Selection::LoadState | Selection::Reset => "hold SELECT".into(),
            Selection::VideoScale => VIDEO_SCALE_NAMES[self.video_scale.load(Ordering::Relaxed) % VIDEO_SCALE_NAMES.len()].into(),
            Selection::Volume => format!("{:.0}%", self.volume.get() * 100.0),
        }
    }

    fn display_rows(&self) -> Vec<(String, String)> {
        ROWS.iter().map(|&sel| (Self::leaf_name(sel).to_string(), self.leaf_value(sel))).collect()
    }
}

impl App for RetroApp {
    /// Re-scans the current console's ROM library every time this
    /// screen is entered, so a file dropped in while the sim's already
    /// running (no restart) shows up the next time you open Retro from
    /// the launcher -- `roms` was otherwise only ever populated once,
    /// at `RetroApp::new`, on program startup.
    fn on_enter(&mut self) {
        self.rescan_roms();
    }

    fn tick(&mut self, input: &Input) {
        self.sync_sample_rate();

        // knob1 is otherwise only the menu's own browse control, and
        // this app has nothing else to spend a press on -- see the
        // module doc comment's "not attempted yet" list for why this,
        // not a long-press gesture `Input` has no concept of, is what
        // gets you back to the menu (Save State/Load State/ROM/etc.)
        // once a game is running full-screen.
        if input.knob1_press && self.loaded_rom_name.is_some() {
            self.menu_visible = !self.menu_visible;
        }

        if self.menu_visible { self.list.navigate_input(input, ROWS.len(), self.nav_speed.get() as i32); }
        let sel = ROWS[self.list.selected];

        // Editing menu rows while the game is full-screen (not
        // visible) would silently change settings from stray knob
        // turns during play -- only live while the menu itself is
        // showing.
        if self.menu_visible && input.knob2 != 0 {
            let sensitivity = self.sensitivity.get();
            match sel {
                Selection::Console => {
                    let cur = CONSOLES.iter().position(|&c| c == self.console).unwrap() as i32;
                    let next = (cur + input.knob2.signum()).rem_euclid(CONSOLES.len() as i32);
                    self.console = CONSOLES[next as usize];
                    self.rescan_roms();
                }
                Selection::Rom => {
                    if !self.roms.is_empty() {
                        let cur = self.rom_index.load(Ordering::Relaxed) as i32;
                        let next = (cur + input.knob2.signum()).rem_euclid(self.roms.len() as i32);
                        self.rom_index.store(next as usize, Ordering::Relaxed);
                    }
                }
                Selection::SaveSlot => {
                    let cur = self.save_slot.load(Ordering::Relaxed) as i32;
                    let next = (cur + input.knob2.signum()).rem_euclid(NUM_SAVE_SLOTS as i32);
                    self.save_slot.store(next as usize, Ordering::Relaxed);
                }
                Selection::VideoScale => {
                    let cur = self.video_scale.load(Ordering::Relaxed) as i32;
                    let next = (cur + input.knob2.signum()).rem_euclid(VIDEO_SCALE_NAMES.len() as i32);
                    self.video_scale.store(next as usize, Ordering::Relaxed);
                }
                Selection::Volume => {
                    let next = (self.volume.get() + accelerate(input.knob2) * sensitivity * 0.01).clamp(0.0, 1.0);
                    self.volume.set(next);
                }
                Selection::SaveState | Selection::LoadState | Selection::Reset => {}
            }
        }
        if self.menu_visible && input.knob2_press {
            match sel {
                Selection::Rom => self.load_selected_rom(),
                Selection::SaveState => self.save_state(),
                Selection::LoadState => self.load_state(),
                Selection::Reset => self.reset(),
                Selection::Console | Selection::SaveSlot | Selection::VideoScale | Selection::Volume => {}
            }
        }

        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32();
        self.last_tick = now;

        if self.running {
            self.apply_pad_input(input);
            // Paced to the console's real ~60.0988Hz refresh rate by
            // elapsed wall-clock time, not "once per `tick()`" --
            // `tick()` itself runs at whatever rate the host UI loop
            // happens to call it (the Slint dev tool's own timer is
            // ~30Hz), and each emulated frame always produces a fixed
            // ~1/60s worth of audio regardless of how much real time
            // actually passed. Calling it at the wrong rate either
            // starves the audio ring buffer (glitchy/crackling audio)
            // or floods it. Capped at 4 frames/tick so a genuinely
            // stalled host (a slow debug build, a dropped frame)
            // catches up gradually instead of free-running dozens of
            // frames at once ("spiral of death").
            // Arcade boards don't all share the NES/SNES family's NTSC
            // rate -- `phosphor_machines`' own `frame_rate_hz()` gives
            // each board's *real* rate (Joust 60.10Hz, Missile Command
            // 61.04Hz, and so on), which matters here for exactly the
            // same reason a wrong rate broke SNES audio earlier: pacing
            // against the wrong period starves or floods the queue.
            let frame_seconds = match self.deck.as_ref() {
                Some(Deck::Arcade(machine)) => (1.0 / machine.frame_rate_hz()) as f32,
                Some(Deck::Gb(..)) => GB_FRAME_SECONDS,
                Some(Deck::NeoGeo(..)) => NEOGEO_FRAME_SECONDS,
                _ => FRAME_SECONDS,
            };
            self.frame_accum = (self.frame_accum + dt.min(0.25)).min(4.0 * frame_seconds);
            let mut frames_run = 0;
            while self.frame_accum >= frame_seconds && frames_run < 4 {
                // Both cores are real, third-party, vendored emulators
                // (see the module doc comment) -- neither is bug-free.
                // `super_sabicom` in particular has a confirmed live
                // one: an HDMA line-counter underflow deep in its own
                // bus timing (`bus.rs`'s `hdma_line_counter -= 1`)
                // that a real controller's actual button-mash timing
                // patterns can trigger during ordinary play, which a
                // synthetic single-button-per-frame test never hit.
                // `catch_unwind` here is what turns "the whole
                // simulator goes down" into "this one game stops with
                // an error" -- a panic mid-step can leave the core's
                // internal state genuinely corrupted, so recovery
                // means dropping the deck entirely (forcing a reload),
                // not trying to keep stepping it.
                let stepped: Result<(), String> = match self.deck.as_mut() {
                    Some(Deck::Nes(deck)) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| deck.clock_frame())) {
                        Ok(Ok(_)) => {
                            // `ControlDeck::frame_buffer` is a fixed
                            // `&[u8; 256*240*4]` RGBA8 buffer -- these
                            // dimensions are the NES's own real,
                            // unchanging output resolution, not
                            // something the crate exposes as a public
                            // constant to read back.
                            const NES_W: usize = 256;
                            const NES_H: usize = 240;
                            self.last_frame = Some((NES_W, NES_H, deck.frame_buffer().to_vec()));
                            let mut queue = self.bridge.queue.lock().unwrap();
                            for &s in deck.audio_samples() {
                                if queue.len() + 1 < MAX_QUEUED_AUDIO_SAMPLES {
                                    queue.push_back(s);
                                    queue.push_back(s);
                                }
                            }
                            Ok(())
                        }
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(panic) => Err(panic_message(panic)),
                    },
                    Some(Deck::Snes(snes)) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| snes.exec_frame(true))) {
                        Ok(()) => {
                            let (w, h, rgba) = snes_frame_to_rgba(snes.frame_buffer());
                            self.last_frame = Some((w, h, rgba));
                            let audio = snes.audio_buffer();
                            // SNES hardware's own fixed 32000Hz -> the
                            // real device rate -- see `LinearResampler`'s
                            // doc comment for why this is the actual "why
                            // is the sound glitchy" fix (NES needs no
                            // equivalent; `ControlDeck::set_sample_rate`
                            // already resamples for it internally).
                            let mut interleaved = Vec::with_capacity(audio.samples.len() * 2);
                            for sample in &audio.samples {
                                interleaved.push(sample.left as f32 / 32768.0);
                                interleaved.push(sample.right as f32 / 32768.0);
                            }
                            let resampled = self.resampler.resample(&interleaved, audio.sample_rate as f32, self.configured_sample_rate.max(1.0), 2);
                            let mut queue = self.bridge.queue.lock().unwrap();
                            for s in resampled {
                                if queue.len() + 1 < MAX_QUEUED_AUDIO_SAMPLES {
                                    queue.push_back(s);
                                }
                            }
                            Ok(())
                        }
                        Err(panic) => Err(panic_message(panic)),
                    },
                    Some(Deck::Arcade(machine)) => {
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| machine.run_frame())) {
                        Ok(()) => {
                            let (w, h, rgba) = arcade_frame_to_rgba(machine.as_ref());
                            self.last_frame = Some((w, h, rgba));
                            // Arcade boards vary wildly in native audio
                            // rate/channel count (mostly mono, a few
                            // stereo -- see `AudioSource::audio_channels`'s
                            // own doc comment) -- normalize to
                            // interleaved stereo before the shared
                            // resampler/queue, same convention NES/SNES
                            // already use.
                            let mut pcm = vec![0i16; 4096];
                            let written = machine.fill_audio(&mut pcm);
                            let channels = machine.audio_channels().max(1) as usize;
                            let mut interleaved = Vec::with_capacity(written * 2 / channels.max(1));
                            for frame in pcm[..written].chunks(channels) {
                                let l = frame[0] as f32 / 32768.0;
                                let r = if channels >= 2 { frame[1] as f32 / 32768.0 } else { l };
                                interleaved.push(l);
                                interleaved.push(r);
                            }
                            let resampled = self.resampler.resample(&interleaved, machine.audio_sample_rate().max(1) as f32, self.configured_sample_rate.max(1.0), 2);
                            let mut queue = self.bridge.queue.lock().unwrap();
                            for s in resampled {
                                if queue.len() + 1 < MAX_QUEUED_AUDIO_SAMPLES {
                                    queue.push_back(s);
                                }
                            }
                            Ok(())
                        }
                            Err(panic) => Err(panic_message(panic)),
                        }
                    }
                    Some(Deck::Gb(device, audio_buf)) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // `do_cycle` steps a small, variable batch of
                        // clock cycles at a time (not a whole frame),
                        // so this drives it until the GPU actually
                        // finishes rendering one -- the real signal a
                        // frame is ready, rather than a hardcoded cycle
                        // count this app would have to keep in sync
                        // with the real GB clock by hand. Capped at
                        // 200_000 iterations (several real frames'
                        // worth even at the smallest real per-call
                        // cycle count) so a core bug that never sets
                        // the GPU-updated flag can't hang this thread
                        // forever.
                        let mut iterations = 0;
                        while !device.check_and_reset_gpu_updated() && iterations < 200_000 {
                            device.do_cycle();
                            iterations += 1;
                        }
                    })) {
                        Ok(()) => {
                            let rgba = rgb24_to_rgba(device.get_gpu_data());
                            self.last_frame = Some((rboy::SCREEN_W, rboy::SCREEN_H, rgba));
                            let mut buf = audio_buf.lock().unwrap();
                            let mut queue = self.bridge.queue.lock().unwrap();
                            for (&l, &r) in buf.0.iter().zip(buf.1.iter()) {
                                if queue.len() + 1 < MAX_QUEUED_AUDIO_SAMPLES {
                                    queue.push_back(l);
                                    queue.push_back(r);
                                }
                            }
                            buf.0.clear();
                            buf.1.clear();
                            Ok(())
                        }
                        Err(panic) => Err(panic_message(panic)),
                    },
                    // No video-timing/raster hardware is emulated
                    // (real hardware paces VBlank off real pixel/line
                    // counters -- see `NEOGEO_FRAME_SECONDS`'s own doc
                    // comment) -- a fixed instruction budget per video
                    // frame stands in for it instead, close to a real
                    // 68000's actual per-frame instruction throughput
                    // at this frame rate. Real YM2610 audio; Metal Slug
                    // 3's own protection chips are decrypted (see
                    // `neogeo_core`'s own module doc comment) -- most
                    // other real cartridges' own protection chips
                    // still aren't, and those will fault instead.
                    Some(Deck::NeoGeo(machine)) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        machine.vblank();
                        for _ in 0..50_000 {
                            if !machine.step() {
                                break;
                            }
                        }
                        for _ in 0..10_000 {
                            machine.step_sound();
                        }
                        let frame = machine.bus.render_frame();
                        let audio = machine.generate_audio_seconds(NEOGEO_FRAME_SECONDS);
                        (frame, audio, machine.audio_sample_rate())
                    })) {
                        Ok(((w, h, rgba), (left, right), native_rate)) => {
                            self.last_frame = Some((w, h, rgba));
                            let mut interleaved = Vec::with_capacity(left.len() * 2);
                            for (l, r) in left.into_iter().zip(right) {
                                interleaved.push(l);
                                interleaved.push(r);
                            }
                            let resampled = self.resampler.resample(&interleaved, native_rate, self.configured_sample_rate.max(1.0), 2);
                            let mut queue = self.bridge.queue.lock().unwrap();
                            for s in resampled {
                                if queue.len() + 1 < MAX_QUEUED_AUDIO_SAMPLES {
                                    queue.push_back(s);
                                }
                            }
                            Ok(())
                        }
                        Err(panic) => Err(panic_message(panic)),
                    },
                    None => Err("no deck loaded".into()),
                };
                match stepped {
                    Ok(()) => {}
                    Err(e) => {
                        self.status = format!("emulation error, reload the ROM: {e}");
                        self.running = false;
                        self.deck = None;
                        self.loaded_rom_name = None;
                        break;
                    }
                }
                self.frame_accum -= frame_seconds;
                frames_run += 1;
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        // Full-screen game view -- the whole point of "press knob1 for
        // the menu" (see `tick`): the game gets the entire device
        // screen, no menu chrome eating into it, matching how a real
        // console/handheld shows gameplay.
        if !self.menu_visible {
            Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK)).draw(fb).ok();
            if let Some((w, h, frame)) = &self.last_frame {
                let stretch = self.video_scale.load(Ordering::Relaxed) == 1;
                blit_frame(fb, frame, *w, *h, 0, 0, WIDTH as i32, HEIGHT as i32, stretch);
            }
            return;
        }

        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(RETRO_BG)).draw(fb).ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, RETRO_TITLE);
        Text::new(&format!("Retro -- {}", self.console.name()), Point::new(16, 30), title).draw(fb).ok();

        let rows = self.display_rows();
        self.list.draw_themed(fb, 16, 56, 22, rows.len(), &rows, RETRO_BG, RETRO_DIM, RETRO_CHIP_BG);

        let video_x = 260;
        let video_y = 44;
        let video_w = WIDTH as i32 - video_x - 16;
        let video_h = HEIGHT as i32 - video_y - 30;
        Rectangle::new(Point::new(video_x, video_y), Size::new(video_w as u32, video_h as u32)).into_styled(PrimitiveStyle::with_stroke(RETRO_DIM, 1)).draw(fb).ok();
        if let Some((w, h, frame)) = &self.last_frame {
            let stretch = self.video_scale.load(Ordering::Relaxed) == 1;
            blit_frame(fb, frame, *w, *h, video_x + 1, video_y + 1, video_w - 2, video_h - 2, stretch);
        } else {
            let dim = MonoTextStyle::new(&SPLEEN_6X12, RETRO_DIM);
            Text::new("no game loaded", Point::new(video_x + 12, video_y + 20), dim).draw(fb).ok();
        }

        let dim = MonoTextStyle::new(&SPLEEN_6X12, RETRO_DIM);
        let hint = if self.loaded_rom_name.is_some() { format!("{} -- SELECT for fullscreen", self.status) } else { self.status.clone() };
        Text::new(&hint, Point::new(16, HEIGHT as i32 - 12), dim).draw(fb).ok();
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows().into_iter().map(|(n, v)| (n, v, false)).collect()
    }
    fn slint_pointer_pick(&mut self,x:f32,_:f32) {
        if !self.menu_visible{return;}
        match x as i32 {
            0..=4=>{self.console=CONSOLES[(x as usize).min(CONSOLES.len()-1)];self.rescan_roms();self.list.selected=1;},
            20=>{if self.loaded_rom_name.is_some(){self.menu_visible=false;}else{self.load_selected_rom();}},
            21=>self.load_selected_rom(),_=>{},
        }
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }

    /// F3 (this OS's real Start/Stop button, see `App::running`'s doc
    /// comment) pauses/resumes the emulator -- the one overlay action
    /// that matters most for a game, reachable without needing the
    /// long-press gesture the original spec assumed (`Input` has no
    /// such concept -- see the module doc comment's "not attempted
    /// yet" list).
    fn running(&self) -> Option<bool> {
        if self.loaded_rom_name.is_some() {
            Some(self.running)
        } else {
            None
        }
    }

    fn toggle_running(&mut self) {
        if self.loaded_rom_name.is_some() {
            self.running = !self.running;
        }
    }

    /// "Totally full screen" means no OS-drawn bottom bar composited
    /// over the game either -- see `Os::run`'s own gating on this.
    fn wants_fullscreen(&self) -> bool {
        !self.menu_visible
    }

    fn transport_action(&self) -> Option<&'static str> {
        self.running().map(|running| if running { "PAUSE" } else { "RESUME" })
    }

    /// Feeds the Slint dev-preview panel the same real video frame the
    /// device screen's `draw()` blits.
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let (frame_w, frame_h, frame_rgba) = match &self.last_frame {
            Some((w, h, rgba)) => (*w as u32, *h as u32, rgba.clone()),
            None => (0, 0, Vec::new()),
        };
        crate::app::SlintExtra::Retro(crate::app::RetroExtra {
            consoles: CONSOLE_NAMES.iter().map(|c| c.to_string()).collect(),
            loaded_name: self.loaded_rom_name.clone().unwrap_or_default(),
            rom_count: self.roms.len() as i32,
            console_name: self.console.name().to_string(),
            rom_name: self.rom_name().to_string(),
            running: self.running,
            menu_visible: self.menu_visible,
            status: self.status.clone(),
            frame_w,
            frame_h,
            frame_rgba,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(RetroAudioProcessor {
            bridge: Arc::clone(&self.bridge),
            bus_out: Arc::clone(&self.bus_out),
            volume: AtomicF32::new(self.volume.get()),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
        }))
    }
}

/// Drains whatever the emulator has queued into this block, padding
/// with silence on underrun (the deck simply hasn't run far enough
/// ahead yet -- e.g. right after loading a ROM, or if the UI thread's
/// frame rate dipped) rather than blocking or glitching. The queue is
/// always interleaved stereo pairs (see `AudioBridge`'s own doc
/// comment) regardless of which console/core produced them.
struct RetroAudioProcessor {
    bridge: Arc<AudioBridge>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// A snapshot taken when the processor was built -- real-time-safe
    /// (no atomics read for a lock-free-adjacent volume that could
    /// drift oddly), matching this app's own convention of applying
    /// `Volume` at construction, not live. Revisit if live volume
    /// automation turns out to matter for this app specifically.
    volume: AtomicF32,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl AudioProcessor for RetroAudioProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        self.bridge.sample_rate.set(sample_rate);
        let channels = channels.max(1);
        let frames = buffer.len() / channels;
        let volume = self.volume.get();
        let mix_level = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut mono_bus = Vec::with_capacity(frames);
        {
            let mut queue = self.bridge.queue.lock().unwrap();
            for frame in buffer.chunks_mut(channels) {
                let l = queue.pop_front().unwrap_or(0.0) * volume;
                let r = queue.pop_front().unwrap_or(0.0) * volume;
                mono_bus.push((l + r) * 0.5);
                if channels >= 2 {
                    frame[0] = l * mix_level;
                    frame[1] = r * mix_level;
                    for extra in frame.iter_mut().skip(2) {
                        *extra = l * mix_level;
                    }
                } else {
                    frame[0] = (l + r) * 0.5 * mix_level;
                }
            }
        }
        *self.bus_out.lock().unwrap() = mono_bus;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> RetroApp {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        RetroApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus)
    }

    /// The actual bug behind "sound is glitchy" while frame rate was
    /// fine: SNES's fixed 32000Hz audio was queued with no resampling
    /// against a 48000Hz consumer, a real 1.5x rate mismatch that
    /// starves the queue on a steady cadence. `LinearResampler` must
    /// produce (within rounding) exactly `out_rate/in_rate` as many
    /// output frames as input frames fed to it, sustained over many
    /// calls (not just the first one, since `pending`/`pos` carrying
    /// state incorrectly across calls would drift over time even if a
    /// single call looked right).
    #[test]
    fn linear_resampler_produces_the_right_output_rate_sustained_over_many_calls() {
        let mut r = LinearResampler::default();
        let in_rate = 32000.0;
        let out_rate = 48000.0;
        let in_frames_per_call = 532; // matches super_sabicom's real per-frame audio chunk size
        let calls = 120; // 2 real seconds' worth at 60Hz
        let mut total_out_frames = 0usize;
        for i in 0..calls {
            // Not silence -- a real varying signal, so a resampling
            // bug that only manifests on non-constant input (e.g. an
            // off-by-one in the interpolation index) would show up.
            let input: Vec<f32> = (0..in_frames_per_call * 2).map(|s| ((i * in_frames_per_call * 2 + s) as f32 * 0.01).sin()).collect();
            let out = r.resample(&input, in_rate, out_rate, 2);
            total_out_frames += out.len() / 2;
        }
        let expected = (calls * in_frames_per_call) as f32 * (out_rate / in_rate);
        let actual = total_out_frames as f32;
        assert!((actual - expected).abs() / expected < 0.01, "expected ~{expected} output frames for {out_rate}/{in_rate} resampling over {calls} calls, got {actual}");
    }

    /// The actual crash report this whole safety net exists for: a
    /// real vendored core (`super_sabicom`) panicking on the main
    /// thread mid-`exec_frame` (a confirmed HDMA line-counter
    /// underflow, `bus.rs:910`, triggered by real controller input
    /// timing) used to take the entire simulator down with it.
    /// `panic_message` is the one piece of that recovery path testable
    /// without reproducing the exact HDMA condition -- both the `&str`
    /// panics core code actually uses and the arithmetic-overflow
    /// panics Rust's own debug-mode overflow checks raise (a plain
    /// `String`, not `&str`) must both extract a real message, and a
    /// non-string payload must degrade to a message, not another
    /// panic.
    #[test]
    fn panic_message_extracts_str_and_string_payloads_and_degrades_gracefully() {
        let str_payload: Box<dyn std::any::Any + Send> = Box::new("attempt to subtract with overflow");
        assert_eq!(panic_message(str_payload), "attempt to subtract with overflow");

        let string_payload: Box<dyn std::any::Any + Send> = Box::new(String::from("custom panic message"));
        assert_eq!(panic_message(string_payload), "custom panic message");

        let opaque_payload: Box<dyn std::any::Any + Send> = Box::new(42i32);
        assert_eq!(panic_message(opaque_payload), "core panicked (no message)");
    }

    #[test]
    fn no_rom_loaded_is_silent_and_reports_not_running() {
        let mut app = new_app();
        assert_eq!(app.running(), None, "no game loaded yet -- F3 should have nothing to act on");
        let mut proc = app.audio_processor().unwrap();
        let mut buffer = vec![1.0f32; 256 * 2];
        proc.process(&mut buffer, 2, 48_000.0);
        assert!(buffer.iter().all(|&s| s == 0.0), "no ROM loaded should mean silence, not leftover buffer contents");
    }

    #[test]
    fn scan_roms_finds_nothing_in_a_missing_directory_without_panicking() {
        let roms = scan_roms(Path::new("/definitely/not/a/real/portamax/roms/path"), &NES_EXTENSIONS);
        assert!(roms.is_empty());
    }

    #[test]
    fn scan_roms_matches_any_of_several_extensions_case_insensitively() {
        let scratch_root = std::env::temp_dir().join(format!("portamax_retro_ext_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch_root);
        std::fs::create_dir_all(&scratch_root).unwrap();
        std::fs::write(scratch_root.join("a.SFC"), b"x").unwrap();
        std::fs::write(scratch_root.join("b.smc"), b"x").unwrap();
        std::fs::write(scratch_root.join("c.txt"), b"x").unwrap();
        let roms = scan_roms(&scratch_root, &SNES_EXTENSIONS);
        assert_eq!(roms.len(), 2, "should match .SFC (case-insensitive) and .smc, but not .txt");
        let _ = std::fs::remove_dir_all(&scratch_root);
    }

    #[test]
    fn save_state_path_is_keyed_by_console_rom_name_and_slot() {
        let a = save_state_path(Console::Nes, "Some Game", 0);
        let b = save_state_path(Console::Nes, "Some Game", 1);
        let c = save_state_path(Console::Nes, "Another Game", 0);
        let d = save_state_path(Console::Snes, "Some Game", 0);
        assert_ne!(a, b, "different slots must not collide");
        assert_ne!(a, c, "different ROMs must not collide");
        assert_ne!(a, d, "different consoles must not collide even with the same ROM name and slot");
        assert!(a.to_string_lossy().contains("Some Game"));
    }

    /// `blit_frame` must never panic regardless of scale mode, source
    /// resolution, or target rectangle size -- covers both consoles'
    /// real resolutions plus the letterboxed "Fit" and "Stretch" paths
    /// a real game screen exercises every frame.
    #[test]
    fn blit_never_panics_in_either_scale_mode_or_resolution() {
        let mut fb = FrameBuffer::new();
        for (src_w, src_h) in [(256, 240), (256, 224)] {
            let frame = vec![128u8; src_w * src_h * 4];
            blit_frame(&mut fb, &frame, src_w, src_h, 10, 10, 300, 200, false);
            blit_frame(&mut fb, &frame, src_w, src_h, 10, 10, 300, 200, true);
            // Degenerate target sizes (e.g. a not-yet-laid-out screen)
            // must not divide by zero or index out of bounds either.
            blit_frame(&mut fb, &frame, src_w, src_h, 0, 0, 1, 1, false);
        }
        // A zero-sized or mismatched-length source must be a no-op,
        // not a panic (the "no frame yet" and "wrong console's stale
        // buffer" cases).
        blit_frame(&mut fb, &[], 0, 0, 0, 0, 100, 100, false);
        blit_frame(&mut fb, &[1, 2, 3], 256, 240, 0, 0, 100, 100, false);
    }

    /// The whole point of "full screen" -- a loaded ROM should
    /// immediately hide the menu, and knob1_press should toggle it
    /// back and forth from there. Skips the load half cleanly when no
    /// ROM is present (this repo intentionally ships without one).
    #[test]
    fn loading_a_rom_goes_fullscreen_and_knob1_toggles_the_menu() {
        let mut app = new_app();
        assert!(app.menu_visible, "must start showing the menu -- nothing else to pick a ROM from yet");
        if app.roms.is_empty() {
            eprintln!("skipping the load half: no ROM in {NES_ROMS_DIR}");
            return;
        }
        app.load_selected_rom();
        assert!(!app.menu_visible, "loading a ROM should go full-screen automatically");
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert!(app.menu_visible, "knob1_press should bring the menu back");
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert!(!app.menu_visible, "knob1_press again should return to full-screen");
    }

    /// `wants_fullscreen` gates whether the OS paints its bottom bar
    /// over the game (see `Os::run`) -- must track `menu_visible`
    /// exactly, in both directions.
    #[test]
    fn wants_fullscreen_tracks_menu_visibility() {
        let mut app = new_app();
        assert!(!app.wants_fullscreen(), "the menu is showing at start -- must not claim the whole screen yet");
        app.menu_visible = false;
        assert!(app.wants_fullscreen());
        app.menu_visible = true;
        assert!(!app.wants_fullscreen());
    }

    /// `on_enter` must actually re-scan the current console's ROM
    /// directory (not just reuse whatever `RetroApp::new` saw at
    /// startup) and keep the current selection by name across it --
    /// exactly what "register new ROMs upon loading [the screen]"
    /// means.
    #[test]
    fn on_enter_rescans_roms_dir_and_keeps_the_current_selection_by_name() {
        let mut app = new_app();
        if app.roms.is_empty() {
            eprintln!("skipping: no ROM in {NES_ROMS_DIR}");
            return;
        }
        let selected_name = app.rom_name().to_string();
        app.roms.clear(); // simulate "this instance hasn't rescanned since startup"
        app.on_enter();
        assert!(!app.roms.is_empty(), "on_enter must repopulate roms from the console's ROMS_DIR, not leave the cleared Vec empty");
        assert_eq!(app.rom_name(), selected_name, "the previously-selected ROM must still be selected by name after a rescan");
    }

    /// The whole point of the Console row: switching it must swap
    /// which ROM library `Rom` browses -- reducing "menu diving" means
    /// this happens right there in the flat top-level list, not behind
    /// a submenu.
    #[test]
    fn switching_console_rescans_into_that_consoles_own_rom_library() {
        let mut app = new_app();
        assert_eq!(app.console, Console::Nes);
        let expected_nes: Vec<String> = scan_roms(Path::new(NES_ROMS_DIR), &NES_EXTENSIONS).into_iter().map(|r| r.name).collect();
        assert_eq!(app.roms.iter().map(|r| r.name.clone()).collect::<Vec<_>>(), expected_nes);

        // Selection::Console is row 0.
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.console, Console::Snes, "one knob2 tick on the Console row should switch NES -> SNES");
        let expected_snes: Vec<String> = scan_roms(Path::new(SNES_ROMS_DIR), &SNES_EXTENSIONS).into_iter().map(|r| r.name).collect();
        assert_eq!(
            app.roms.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            expected_snes,
            "switching to SNES must rescan into SNES_ROMS_DIR, not keep NES's list"
        );

        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.console, Console::Arcade, "one more tick should switch SNES -> Arcade");
        let expected_arcade: Vec<String> = scan_arcade_roms(Path::new(ARCADE_ROMS_DIR)).into_iter().map(|r| r.name).collect();
        assert_eq!(
            app.roms.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            expected_arcade,
            "switching to Arcade must rescan the phosphor_machines registry against ARCADE_ROMS_DIR, not keep SNES's list"
        );

        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.console, Console::Gb, "one more tick should switch Arcade -> Game Boy");
        let expected_gb: Vec<String> = scan_roms(Path::new(GB_ROMS_DIR), &GB_EXTENSIONS).into_iter().map(|r| r.name).collect();
        assert_eq!(
            app.roms.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            expected_gb,
            "switching to Game Boy must rescan GB_ROMS_DIR, not keep Arcade's list"
        );

        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.console, Console::NeoGeo, "one more tick should switch Game Boy -> Neo Geo");
        let expected_neogeo: Vec<String> = scan_neogeo_roms(Path::new(NEOGEO_ROMS_DIR)).into_iter().map(|r| r.name).collect();
        assert_eq!(
            app.roms.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
            expected_neogeo,
            "switching to Neo Geo must rescan NEOGEO_ROMS_DIR, not keep Game Boy's list"
        );

        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.console, Console::Nes, "wraps back to NES after all five consoles");
    }

    /// `scan_arcade_roms` must actually consult the real
    /// `phosphor_machines` registry (dozens of real machines -- Pac-
    /// Man, Galaga, Donkey Kong, and more) rather than a hardcoded
    /// list, and every entry it does emit must genuinely have a ROM
    /// directory on disk matching one of that machine's own declared
    /// `rom_names` (not just any file that happens to be there).
    #[test]
    fn scan_arcade_roms_only_lists_registered_machines_with_a_real_rom_directory_present() {
        let scratch_root = std::env::temp_dir().join(format!("portamax_retro_arcade_scan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch_root);
        std::fs::create_dir_all(&scratch_root).unwrap();

        assert!(scan_arcade_roms(&scratch_root).is_empty(), "an empty directory must yield no machines, regardless of how many are registered");

        let all_machines = phosphor_machines::registry::all();
        assert!(!all_machines.is_empty(), "the registry itself must have real machines -- otherwise this whole test proves nothing");
        let first = all_machines[0];
        std::fs::create_dir_all(scratch_root.join(first.rom_names[0])).unwrap();

        let found = scan_arcade_roms(&scratch_root);
        assert_eq!(found.len(), 1, "exactly the one machine with a matching directory should be found");
        assert_eq!(found[0].name, first.name);
        assert!(found[0].machine.is_some_and(|m| std::ptr::eq(m, first)), "the found entry must carry the exact registry entry it matched, for load_selected_rom to use later");

        let _ = std::fs::remove_dir_all(&scratch_root);
    }

    #[test]
    fn toggle_running_is_a_no_op_with_no_game_loaded() {
        let mut app = new_app();
        app.toggle_running();
        assert_eq!(app.running(), None, "F3 must do nothing until a game is actually loaded");
    }

    #[test]
    fn transport_action_reads_pause_resume_not_generic_play_stop() {
        let app = new_app();
        assert_eq!(app.transport_action(), None);
    }

    /// If a real NES ROM is sitting in `roms/nes` (user-supplied,
    /// never committed -- see `.gitignore`), actually load it and run
    /// it for real: confirms `tetanes_core` is wired up correctly end
    /// to end -- ROM loading, frame stepping, video output, audio
    /// output, save state round-trip -- not just that the surrounding
    /// Rust compiles. Skips (doesn't fail) when no ROM is present.
    #[test]
    fn a_real_nes_rom_actually_runs_if_one_is_present() {
        let mut app = new_app();
        if app.roms.is_empty() {
            eprintln!("skipping: no ROM in {NES_ROMS_DIR} (this is expected in a fresh checkout)");
            return;
        }
        app.load_selected_rom();
        assert!(app.deck.is_some(), "a real ROM file must load successfully: {}", app.status);
        assert_eq!(app.running(), Some(true), "loading a ROM should leave it running");

        let mut proc = app.audio_processor().unwrap();
        let mut saw_audio = false;
        let mut saw_nonblank_frame = false;
        for _ in 0..90 {
            // `tick`'s own real-time pacing (see its doc comment) only
            // actually steps a frame once enough wall-clock time has
            // elapsed -- a tight loop with no delay between calls
            // would never accumulate a single frame's worth. A real
            // sleep is the only honest way to exercise that path.
            std::thread::sleep(std::time::Duration::from_millis(17));
            // Real input driving it, not just idle frames -- an idle
            // title screen can render the exact same frame forever,
            // which wouldn't actually prove frame-stepping is doing
            // anything. Holding Start is real enough to guarantee
            // *something* on screen changes over 90 frames (1.5s).
            app.tick(&Input { grid: std::array::from_fn(|i| i == 7), ..Default::default() });
            let mut buffer = vec![0.0f32; 256 * 2];
            proc.process(&mut buffer, 2, 48_000.0);
            if buffer.iter().any(|&s| s != 0.0) {
                saw_audio = true;
            }
            if let Some((_, _, frame)) = &app.last_frame {
                if frame.iter().any(|&b| b != frame[0]) {
                    saw_nonblank_frame = true;
                }
            }
        }
        assert!(app.last_frame.is_some(), "frame-stepping should have produced real video output");
        assert!(saw_nonblank_frame, "the rendered frame should have actual image content, not a single flat color");
        assert!(saw_audio, "a running game should produce real audible output through the mix");

        // Save state round-trip against the real deck.
        app.save_state();
        assert!(app.status.starts_with("saved"), "save_state should have succeeded: {}", app.status);
        app.load_state();
        assert!(app.status.starts_with("loaded"), "load_state should have succeeded: {}", app.status);

        // Clean up the save file this test just wrote so repeated runs
        // don't accumulate slot files.
        let rom_name = app.loaded_rom_name.clone().unwrap();
        let _ = std::fs::remove_file(save_state_path(Console::Nes, &rom_name, 0));
    }

    /// The same real end-to-end exercise as the NES test above, but for
    /// SNES specifically, over a much longer real session (10s, ~600
    /// frames) with varying input every frame (not just one button
    /// held) and a pause/resume + save/load cycle in the middle --
    /// closer to how a real play session actually exercises the app
    /// than a short fixed-input loop, since the user's own report was
    /// "glitchy, then crashed" during real play, not on load. Skips
    /// (doesn't fail) when no SNES ROM is present.
    #[test]
    fn a_real_snes_rom_survives_a_longer_realistic_session() {
        let mut app = new_app();
        app.console = Console::Snes;
        app.rescan_roms();
        if app.roms.is_empty() {
            eprintln!("skipping: no ROM in {SNES_ROMS_DIR} (this is expected in a fresh checkout)");
            return;
        }
        app.load_selected_rom();
        assert!(app.deck.is_some(), "a real SNES ROM file must load successfully: {}", app.status);
        assert_eq!(app.running(), Some(true), "loading a ROM should leave it running");

        let mut proc = app.audio_processor().unwrap();
        let mut saw_audio = false;
        let mut saw_nonblank_frame = false;
        let mut max_frame_time = std::time::Duration::ZERO;
        for i in 0..600 {
            std::thread::sleep(std::time::Duration::from_millis(17));
            // Cycle through every mapped SNES pad index across the
            // session (D-Pad, face buttons, shoulders, Start/Select)
            // instead of holding one button the whole time -- a real
            // play session presses different combinations every frame,
            // and `apply_pad_input`'s per-console branch is exactly
            // the newest, least-exercised code path here.
            let held = i % 12;
            let before = Instant::now();
            app.tick(&Input { grid: std::array::from_fn(|pad| pad == held), ..Default::default() });
            max_frame_time = max_frame_time.max(before.elapsed());

            if i == 200 {
                app.toggle_running(); // pause mid-session
                assert_eq!(app.running(), Some(false));
            }
            if i == 220 {
                app.toggle_running(); // resume
                assert_eq!(app.running(), Some(true));
            }
            if i == 300 {
                app.save_state();
                assert!(app.status.starts_with("saved"), "mid-session save_state should have succeeded: {}", app.status);
            }
            if i == 320 {
                app.load_state();
                assert!(app.status.starts_with("loaded"), "mid-session load_state should have succeeded: {}", app.status);
            }

            let mut buffer = vec![0.0f32; 256 * 2];
            proc.process(&mut buffer, 2, 48_000.0);
            if buffer.iter().any(|&s| s != 0.0) {
                saw_audio = true;
            }
            if let Some((_, _, frame)) = &app.last_frame {
                if frame.iter().any(|&b| b != frame[0]) {
                    saw_nonblank_frame = true;
                }
            }
        }
        eprintln!("worst-case single tick() took {max_frame_time:?} (budget: 16.64ms to stay real-time on a 60Hz host loop)");
        assert!(app.last_frame.is_some(), "frame-stepping should have produced real video output");
        assert!(saw_nonblank_frame, "the rendered frame should have actual image content, not a single flat color");
        assert!(saw_audio, "a running game should produce real audible output through the mix");

        let rom_name = app.loaded_rom_name.clone().unwrap();
        let _ = std::fs::remove_file(save_state_path(Console::Snes, &rom_name, 0));
    }

    /// The same real end-to-end exercise as the NES/SNES tests above,
    /// but for Game Boy/Game Boy Color via `rboy`: real ROM load, real
    /// frame stepping (through the `do_cycle`/`check_and_reset_gpu_
    /// updated` loop in `tick`'s `Deck::Gb` arm), real video, real
    /// audio (through `GbAudioSink`), and a real battery-SRAM save/load
    /// round-trip (`dumpram`/`loadram`, not a full save state -- `rboy`
    /// has no on-demand snapshot API, so this exercises the same
    /// mechanism a real GB cartridge's own battery-backed SRAM would).
    /// Skips (doesn't fail) when no GB ROM is present.
    #[test]
    fn a_real_gb_rom_actually_runs_if_one_is_present() {
        let mut app = new_app();
        app.console = Console::Gb;
        app.rescan_roms();
        if app.roms.is_empty() {
            eprintln!("skipping: no ROM in {GB_ROMS_DIR} (this is expected in a fresh checkout)");
            return;
        }
        app.load_selected_rom();
        assert!(app.deck.is_some(), "a real Game Boy ROM file must load successfully: {}", app.status);
        assert_eq!(app.running(), Some(true), "loading a ROM should leave it running");

        let mut proc = app.audio_processor().unwrap();
        let mut saw_audio = false;
        let mut saw_nonblank_frame = false;
        for i in 0..90 {
            std::thread::sleep(std::time::Duration::from_millis(17));
            // Cycle through the mapped GB pads (D-Pad/A/B/Select/Start)
            // instead of holding one button the whole session, same
            // reasoning as the SNES test: exercises the edge-triggered
            // keydown/keyup diffing in `apply_pad_input`'s Gb arm, not
            // just a single held key.
            let held = i % 8;
            app.tick(&Input { grid: std::array::from_fn(|pad| pad == held), ..Default::default() });
            let mut buffer = vec![0.0f32; 256 * 2];
            proc.process(&mut buffer, 2, 48_000.0);
            if buffer.iter().any(|&s| s != 0.0) {
                saw_audio = true;
            }
            if let Some((_, _, frame)) = &app.last_frame {
                if frame.iter().any(|&b| b != frame[0]) {
                    saw_nonblank_frame = true;
                }
            }
        }
        assert!(app.last_frame.is_some(), "frame-stepping should have produced real video output");
        assert!(saw_nonblank_frame, "the rendered frame should have actual image content, not a single flat color");
        assert!(saw_audio, "a running game should produce real audible output through the mix");

        app.save_state();
        assert!(app.status.starts_with("saved"), "save_state should have succeeded: {}", app.status);
        app.load_state();
        assert!(app.status.starts_with("loaded"), "load_state should have succeeded: {}", app.status);

        let rom_name = app.loaded_rom_name.clone().unwrap();
        let _ = std::fs::remove_file(save_state_path(Console::Gb, &rom_name, 0));
    }

    /// `detect_neogeo_protection` must recognize the real cartridge
    /// folder name this project's own `roms/neogeo/metalslug3`
    /// convention actually uses -- not just the MAME set name --
    /// since an earlier version silently fell back to no protection at
    /// all for exactly that folder name, undetected until traced by
    /// hand.
    #[test]
    fn detect_neogeo_protection_recognizes_the_real_metalslug3_folder_name() {
        assert_eq!(detect_neogeo_protection("metalslug3", 0x800000), crate::apps::neogeo_core::Protection::SmaMslug3, "the real folder name this repo's roms/neogeo/ convention uses, with a real full-size encrypted P2, must be detected");
        assert_eq!(detect_neogeo_protection("mslug3", 0x800000), crate::apps::neogeo_core::Protection::SmaMslug3, "the real MAME set name must also be detected");
        assert_eq!(detect_neogeo_protection("MetalSlug3", 0x800000), crate::apps::neogeo_core::Protection::SmaMslug3, "detection must be case-insensitive");
        assert_eq!(detect_neogeo_protection("kof98", 0x800000), crate::apps::neogeo_core::Protection::None, "an unrelated cartridge must not be misdetected as needing Metal Slug 3's own protection");
        assert_eq!(
            detect_neogeo_protection("metalslug3", 0x400000),
            crate::apps::neogeo_core::Protection::None,
            "a folder named like Metal Slug 3 but shaped like a pre-decrypted dump (a single 4MB P2, not the real 8MB encrypted pg1+pg2 pair) must not have this project's own SMA decryption re-applied to already-decrypted data"
        );
    }

    /// The same real end-to-end exercise as the other consoles' tests
    /// above, but for Neo Geo: real cartridge-folder scanning
    /// (`scan_neogeo_roms`), real multi-file ROM loading
    /// (`load_neogeo_cartridge`), and real frame stepping through
    /// `tick`'s `Deck::NeoGeo` arm (68000 + Z80 execution, VBlank
    /// delivery, fix-layer rendering, real YM2610 audio). Unlike every
    /// other console's equivalent test, this doesn't assert save states
    /// work (not supported yet -- checked explicitly, since that's
    /// real, current, documented behavior, not an oversight). Skips
    /// (doesn't fail) when no cartridge folder is present.
    #[test]
    fn a_real_neogeo_cartridge_actually_runs_if_one_is_present() {
        let mut app = new_app();
        app.console = Console::NeoGeo;
        app.rescan_roms();
        if app.roms.is_empty() {
            eprintln!("skipping: no cartridge folder in {NEOGEO_ROMS_DIR} (this is expected in a fresh checkout)");
            return;
        }
        app.load_selected_rom();
        assert!(app.deck.is_some(), "a real Neo Geo cartridge folder must load successfully: {}", app.status);
        assert_eq!(app.running(), Some(true), "loading a cartridge should leave it running");

        let mut proc = app.audio_processor().unwrap();
        let mut saw_audio = false;
        for i in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(17));
            app.tick(&Input { grid: std::array::from_fn(|pad| pad == i % 8), ..Default::default() });
            let mut buffer = vec![0.0f32; 256 * 2];
            proc.process(&mut buffer, 2, 48_000.0);
            if buffer.iter().any(|&s| s != 0.0) {
                saw_audio = true;
            }
            if app.deck.is_none() {
                // The known PVC-protection limitation (see this
                // module's own doc comment) surfaces as exactly this
                // app's normal "emulation error, reload the ROM" path,
                // same as any other core's real crash -- not a test
                // failure, since this is documented, expected behavior
                // for a cart like Metal Slug 3 well within 30 frames.
                eprintln!("deck stopped early (expected for PVC-protected carts): {}", app.status);
                break;
            }
        }
        assert!(app.last_frame.is_some(), "frame-stepping should have produced real video output before any crash");
        // Not asserted as a hard failure: a real cart's sound driver
        // might not have triggered anything audible in these first 30
        // frames (e.g. still in a silent boot/init sequence), so this
        // is a soft signal logged for visibility, not a correctness
        // requirement the way video output is.
        eprintln!("saw real YM2610 audio through the mix: {saw_audio}");

        // Metal Slug 3 specifically is expected to hit its PVC
        // limitation and crash out (see this module's own doc
        // comment) somewhere within this test's 30-frame budget, so
        // the deck may already be gone -- only check the "not
        // supported" message while it's still running; a cart that
        // survives the whole loop should report it explicitly rather
        // than silently succeeding or writing a corrupt file.
        if app.deck.is_some() {
            app.save_state();
            assert!(app.status.contains("aren't supported"), "save states must report as unsupported, not silently succeed or produce a corrupt file: {}", app.status);
        }
    }

    /// Not part of the normal test suite (`#[ignore]`d): dumps an
    /// actual rendered frame from a real, running Metal Slug 3 session
    /// to disk as visual proof the video pipeline (fix layer + sprites
    /// + CMC42-decrypted graphics + real input) produces a real image,
    /// not just "doesn't crash". Run explicitly with
    /// `cargo test --ignored dump_a_real_neogeo_frame -- --nocapture`
    /// and open the path it prints.
    #[test]
    #[ignore]
    fn dump_a_real_neogeo_frame_to_disk_if_a_cartridge_is_present() {
        let mut app = new_app();
        app.console = Console::NeoGeo;
        app.rescan_roms();
        if app.roms.is_empty() {
            eprintln!("skipping: no cartridge folder in {NEOGEO_ROMS_DIR}");
            return;
        }
        app.load_selected_rom();
        assert!(app.deck.is_some(), "cartridge must load: {}", app.status);

        // Tap Start every so often (real pad 7, per `apply_pad_input`'s
        // Neo Geo mapping) to push past any attract-mode/title screen
        // and into real gameplay, same as a player would.
        for frame in 0..900 {
            std::thread::sleep(std::time::Duration::from_millis(17));
            let press_start = frame % 60 == 0;
            app.tick(&Input { grid: std::array::from_fn(|pad| pad == 7 && press_start), ..Default::default() });
            if app.deck.is_none() {
                eprintln!("deck stopped early at frame {frame}: {}", app.status);
                break;
            }
            if frame % 100 == 0 {
                if let Some((w, h, rgba)) = &app.last_frame {
                    let non_black = rgba.chunks_exact(4).filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0).count();
                    eprintln!("frame {frame}: {w}x{h}, {non_black} non-black pixels of {}", w * h);
                }
                if let Some(Deck::NeoGeo(machine)) = app.deck.as_ref() {
                    let (vram, pal) = machine.bus.debug_nonzero_counts();
                    eprintln!("  nonzero vram={vram} palette={pal}, pc=0x{:06X}", machine.cpu.pc);
                }
            }
        }

        let (w, h, rgba) = app.last_frame.clone().expect("a real cartridge must have produced at least one frame");
        let out_path = std::env::temp_dir().join("neogeo_live_frame.png");
        let file = std::fs::File::create(&out_path).expect("create output file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer.write_image_data(&rgba).expect("write PNG data");
        eprintln!("wrote a real running Metal Slug 3 frame to {}", out_path.display());
    }

    /// Diagnoses the real, live finding from
    /// `dump_a_real_neogeo_frame_to_disk_if_a_cartridge_is_present`'s own
    /// output (an all-black frame; VRAM gets one batch of nonzero writes
    /// then goes static; the CPU's PC sits at the exact same BIOS address
    /// for hundreds of real frames in a row): drives the same real
    /// cartridge to that same stuck point, then dumps the raw opcode
    /// words around PC plus every register, and single-steps the CPU a
    /// short run printing PC before/after each step, so the actual spin
    /// loop (and what condition it's testing) can be read off directly
    /// instead of guessed at. `#[ignore]`d for the same reason as its
    /// sibling above -- run explicitly with
    /// `cargo test --bin portamax-sim diagnose_neogeo_stuck_pc -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnose_neogeo_stuck_pc_if_a_cartridge_is_present() {
        use m68k::AddressBus;

        let mut app = new_app();
        app.console = Console::NeoGeo;
        app.rescan_roms();
        if app.roms.is_empty() {
            eprintln!("skipping: no cartridge folder in {NEOGEO_ROMS_DIR}");
            return;
        }
        app.load_selected_rom();
        assert!(app.deck.is_some(), "cartridge must load: {}", app.status);

        // 150 real frames (with periodic Start presses) is well past the
        // point the earlier run showed PC had already frozen (by frame 100).
        for frame in 0..150 {
            std::thread::sleep(std::time::Duration::from_millis(17));
            let press_start = frame % 60 == 0;
            app.tick(&Input { grid: std::array::from_fn(|pad| pad == 7 && press_start), ..Default::default() });
            if app.deck.is_none() {
                eprintln!("deck stopped early at frame {frame}: {}", app.status);
                return;
            }
        }

        let Some(Deck::NeoGeo(machine)) = app.deck.as_mut() else {
            panic!("deck must still be Neo Geo after 150 frames: {}", app.status);
        };
        let sr = machine.cpu.get_sr();
        eprintln!("stuck at pc=0x{:06X}, sr=0x{sr:04X} (interrupt mask={})", machine.cpu.pc, (sr >> 8) & 7);
        for r in 0..8 {
            eprintln!("  d{r}=0x{:08X}  a{r}=0x{:08X}", machine.cpu.d(r), machine.cpu.a(r));
        }
        let pc = machine.cpu.pc;
        eprintln!("opcode words around pc:");
        for off in [-8i32, -6, -4, -2, 0, 2, 4, 6, 8] {
            let addr = (pc as i64 + off as i64) as u32;
            eprintln!("  [{off:+}] 0x{addr:06X}: {:04X}", machine.bus.read_word(addr));
        }

        eprintln!("single-stepping 30 instructions from the stuck pc:");
        for i in 0..30 {
            let before = machine.cpu.pc;
            let ok = machine.step();
            eprintln!(
                "  step {i}: pc 0x{before:06X} -> 0x{:06X}  d0=0x{:08X} d1=0x{:08X} (ok={ok})",
                machine.cpu.pc,
                machine.cpu.d(0),
                machine.cpu.d(1)
            );
            if !ok {
                eprintln!("  CPU halted/faulted -- stopping early");
                break;
            }
        }

        // The spin is `TST.W $10FE8C; BNE` (decoded from the opcode
        // dump above): the loop exits only when that work-RAM word
        // reads zero. Firing a fresh VBlank (the same call `tick`
        // makes once per real frame) and then single-stepping a full
        // real frame's worth of instructions (50,000, matching the
        // per-frame budget in this file's own `tick`) tells us whether
        // the level-1 interrupt is actually ever taken from inside this
        // tight loop, and whether that word ever changes at all.
        let watched = 0x0010FE8Cu32;
        eprintln!("$10FE8C = 0x{:04X} before a fresh vblank", machine.bus.read_word(watched));
        machine.vblank();
        let mut left_loop_at = None;
        let mut value_changed_at = None;
        let initial_value = machine.bus.read_word(watched);
        for i in 0..50_000u32 {
            let pc_before = machine.cpu.pc;
            if !machine.step() {
                eprintln!("  CPU halted/faulted at step {i}");
                break;
            }
            if left_loop_at.is_none() && pc_before != 0xC18714 && pc_before != 0xC1871A {
                left_loop_at = Some((i, pc_before));
            }
            if value_changed_at.is_none() && machine.bus.read_word(watched) != initial_value {
                value_changed_at = Some((i, machine.bus.read_word(watched)));
            }
        }
        eprintln!("after one fresh vblank + 50,000 steps:");
        eprintln!("  pc ended at 0x{:06X}", machine.cpu.pc);
        eprintln!("  left the 0xC18714/0xC1871A loop? {:?}", left_loop_at);
        eprintln!("  $10FE8C changed? {:?} (started at 0x{initial_value:04X})", value_changed_at);

        // Now trace exactly what the main loop actually does once that
        // wait clears: every distinct PC visited across several more
        // real frames' worth of budget, in order of first visit, plus
        // palette/VRAM counts before and after -- this tells us whether
        // real per-frame game logic is running (a wide footprint of
        // addresses, eventually touching palette RAM) or whether the
        // CPU is confined to a tiny loop doing nothing of substance.
        let (vram_before, pal_before) = machine.bus.debug_nonzero_counts();
        let mut seen = std::collections::BTreeSet::new();
        let mut first_visits: Vec<u32> = Vec::new();
        for frame in 0..5 {
            machine.vblank();
            for _ in 0..50_000u32 {
                let pc = machine.cpu.pc;
                if seen.insert(pc) && first_visits.len() < 200 {
                    first_visits.push(pc);
                }
                if !machine.step() {
                    eprintln!("  CPU halted/faulted during trace frame {frame}");
                    break;
                }
            }
        }
        let (vram_after, pal_after) = machine.bus.debug_nonzero_counts();
        eprintln!("across 5 more real frames:");
        eprintln!("  distinct pc addresses visited: {}", seen.len());
        eprintln!("  pc range: 0x{:06X} .. 0x{:06X}", seen.iter().next().copied().unwrap_or(0), seen.iter().next_back().copied().unwrap_or(0));
        eprintln!("  vram nonzero: {vram_before} -> {vram_after}, palette nonzero: {pal_before} -> {pal_after}");
        eprintln!("  first {} distinct pcs visited, in order:", first_visits.len());
        for chunk in first_visits.chunks(8) {
            let line: Vec<String> = chunk.iter().map(|a| format!("0x{a:06X}")).collect();
            eprintln!("    {}", line.join(" "));
        }
        eprintln!("REG_SOUND reply byte (0x320000) = 0x{:02X}", machine.bus.read_byte(0x320000));
    }

    /// No real arcade ROM sets ship with this repo (same legal reasons
    /// as NES/SNES), but `phosphor_machines`' own `MachineEntry::
    /// create_bare` builds a real, fully wired machine with zero-filled
    /// ROM instead of failing -- real hardware structs and devices,
    /// just nothing meaningful for the (unimplemented, all-zero) CPU to
    /// execute. That's exactly enough to drive this app's actual
    /// `Deck::Arcade` pipeline end to end (`run_frame`, `render_frame`
    /// through `arcade_frame_to_rgba`, `fill_audio`, `input_controls`/
    /// `handle_input`) against every one of the dozens of real
    /// registered boards, with no ROM legality question at all --
    /// exactly the coverage `a_real_..._rom_...` above gets from a real
    /// file, generalized to the entire registry instead of one game.
    #[test]
    fn every_registered_arcade_machine_runs_bare_without_panicking() {
        let all_machines = phosphor_machines::registry::all();
        assert!(!all_machines.is_empty(), "the registry itself must have real machines -- otherwise this test proves nothing");

        for entry in all_machines {
            let mut machine = (entry.create_bare)();
            for frame in 0..10 {
                machine.run_frame();
                let (w, h, rgba) = arcade_frame_to_rgba(machine.as_ref());
                assert!(w > 0 && h > 0, "{}: display_size() must be nonzero", entry.name);
                assert_eq!(rgba.len(), w * h * 4, "{}: arcade_frame_to_rgba's output must match its own reported dimensions", entry.name);

                let mut pcm = vec![0i16; 4096];
                let written = machine.fill_audio(&mut pcm);
                assert!(written <= pcm.len(), "{}: fill_audio must not report writing more than the buffer it was given", entry.name);

                // Real input, not just idle frames -- exercises
                // `apply_arcade_pad_input`'s generic control-kind
                // mapping (D-Pad/Coin/Start/Action/Button) against
                // every machine's own declared control set, alternating
                // pads across frames the same way the SNES session test
                // above cycles through its 12.
                let input = Input { grid: std::array::from_fn(|pad| pad == frame % 16), ..Default::default() };
                apply_arcade_pad_input(machine.as_mut(), &input);
            }
        }
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(crate::apps::retro::RetroApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
