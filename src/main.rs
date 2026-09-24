//! Portamax software simulation harness.
//!
//! No microphone input — the Synth app is its own sound source, generated
//! straight into the output callback and shaped by an "AI" stub before it
//! reaches the speakers/line out.
//!
//! This stands in for the STM32N6 audio path so you can validate the
//! software architecture — buffer sizing, control-rate/audio-rate bridging,
//! and where the NPU inference call sits in the signal chain — before any
//! hardware exists.
//!
//! Next steps once this runs cleanly:
//!   1. Swap the hand-rolled oscillators/filter for an infinitedsp graph.
//!   2. Replace `run_inference` with a real onnxruntime call and measure
//!      how much it costs inside the audio callback.
//!   3. Once STM32Cube.AI validation numbers exist for the real model,
//!      compare them against what you measure here.

mod app;
mod apps;
mod arpeggiator;
mod audio;
mod audio_bus;
mod audio_devices;
mod clouds_ffi;
mod controller;
mod display;
mod led_output;
mod manifest;
mod midi_map;
mod mixer_bus;
mod modbus;
mod os;
mod paramlist;
mod plaits_ffi;
mod registry;
mod spleen_fonts;
mod startup_logo;
mod theme;
mod util;

use app::{App, Input};
use apps::prism::PrismCcTargets;
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use controller::ControllerState;
use midir::{Ignore, MidiInput};
use mixer_bus::MixerBus;
use modbus::ModBus;
use os::Os;
use registry::Registry;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use util::AtomicF32;

/// Where installed apps live: `apps/<name>/manifest.toml` (folder) or
/// `apps/<name>.toml` (single file) — see manifest.rs.
const APPS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/apps");

/// The two circled encoders/buttons on your chart = knob 1/2. The pad/
/// top-button note tables (`GRID_NOTES`/`TOP_NOTES`) now live in
/// controller.rs, shared with LED output -- see its doc comment.
const KNOB1_CC: u8 = 77;
const KNOB2_CC: u8 = 78;
const KNOB1_BUTTON_CC: u8 = 108;
const KNOB2_BUTTON_CC: u8 = 109;
/// Best guess at Push 2's "Setup"/menu button, next to the hamburger icon
/// in your chart -- not one of the controls you circled, so treat this as
/// unverified. If it doesn't return you to the launcher, watch for
/// "midi: unmapped CC ..." while pressing the button you want and tell me
/// the number.
const HOME_CC: u8 = 79;

/// Push 2's touch-sensitive encoders send Note On the instant you
/// touch one (and Note Off on release) on very low note numbers --
/// well below any real keyboard's lowest key -- so resting a finger
/// on a knob was triggering the fallback keyboard mapping below.
/// MIDI 21 (A0) is the bottom of a full 88-key piano; nothing a real
/// musical keyboard would ever send should fall below it.
const MIN_FALLBACK_NOTE: u8 = 21;

/// Any note that isn't one of the 16 pads/4 top buttons `GRID_NOTES`/
/// `TOP_NOTES` were hand-tuned for still plays *something* instead of
/// being silently dropped -- wraps the whole 128-note MIDI range
/// across the 16 pads, so any other connected keyboard (or a Push 2
/// pad outside that one picked region) works as a musical keyboard
/// too. Two real notes 16 apart alias onto the same pad; an honest
/// limit of a 16-pad control surface, not worth a bigger redesign for.
/// Returns None below `MIN_FALLBACK_NOTE` -- see its doc comment.
fn fallback_grid_pad(note: u8) -> Option<usize> {
    if note < MIN_FALLBACK_NOTE {
        return None;
    }
    Some(note as usize % 16)
}

/// Prism's macro knobs, direct-mapped from fixed CC numbers -- General
/// Purpose Controllers 20-25, deliberately clear of the Push 2 region
/// above (77-109) and CC1. Each one sets the *same* atomic
/// `apps::prism::Params` reads (see `PrismCcTargets`), scaled from
/// 0..127 to that knob's real range -- Activity/Preset are left out,
/// see `PrismCcTargets`'s own doc comment for why.
const PRISM_TIME_CC: u8 = 20;
const PRISM_REPEATS_CC: u8 = 21;
const PRISM_SHAPE_CC: u8 = 22;
const PRISM_FILTER_CC: u8 = 23;
const PRISM_MIX_CC: u8 = 24;
const PRISM_SPACE_CC: u8 = 25;

/// Push 2 encoders in User Mode send relative "2's complement" ticks:
/// 1..63 = clockwise by that many, 65..127 = counter-clockwise (127 = -1).
fn decode_relative(value: u8) -> i32 {
    if value < 64 {
        value as i32
    } else {
        value as i32 - 128
    }
}

/// Stand-in for the eventual STM32Cube.AI inference call.
/// Called once per audio block. Replace the body with a real model call
/// when you have one, and time it — that's the number that tells you
/// whether your buffer size and expected NPU throughput are compatible.
pub(crate) fn run_inference(block: &mut [f32]) {
    // no-op for now — identity pass-through
    let _ = block;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cutoff = Arc::new(AtomicF32::new(1000.0));
    // Global knob sensitivity -- a Settings menu item now, not a code
    // constant (see apps/settings.rs), shared by every app that reads it.
    let sensitivity = Arc::new(AtomicF32::new(0.1));
    // How many raw encoder ticks knob1 needs before a list moves by one
    // row (see paramlist.rs) -- independent of `sensitivity` above, since
    // list navigation is a discrete step-per-detent action, not a scaled
    // continuous edit.
    let nav_speed = Arc::new(AtomicF32::new(3.0));
    let show_cpu = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let midi_map = Arc::new(midi_map::MidiMap::new());
    // Settings' live color wheel -- see theme.rs. Threaded through
    // Registry::new like every other shared control so Settings can
    // edit it, but this binary's own on-device (embedded_graphics)
    // renderer doesn't read it back yet -- only the Slint live
    // prototype (examples/slint_home_live.rs) actually recolors from
    // it today, so Settings' Hue/Saturation/Brightness rows are real
    // and functional here but not yet visible in this bin's own draw().
    let accent = Arc::new(theme::ThemeColor::new(theme::ACCENT_DEFAULT_HUE, theme::ACCENT_DEFAULT_SAT, theme::ACCENT_DEFAULT_VAL));
    let background = Arc::new(theme::ThemeColor::new(theme::BG_DEFAULT_HUE, theme::BG_DEFAULT_SAT, theme::BG_DEFAULT_VAL));
    // Final gain stage applied to the whole mix, after every app's
    // processor has summed in -- see audio.rs and apps/mixer.rs.
    let master_volume = Arc::new(AtomicF32::new(1.0));
    let controller = Arc::new(ControllerState::new());
    // Prism's macro knobs, shared directly with a fixed set of MIDI CC
    // numbers (see PRISM_*_CC and handle_midi_message) -- the same
    // atomics apps::prism::Params reads, threaded into Registry::new
    // below, so a hardware CC knob and the in-app knob are one and
    // the same value, not two that need reconciling.
    let prism_cc = Arc::new(PrismCcTargets::new());

    // Shared registry of modulation targets any app can expose (e.g.
    // Plaits' Harmonics) for a modulation-source app (Pam's) to drive —
    // see modbus.rs. Created before the MIDI listener below since it
    // also needs a handle (a user-mapped CC can drive any of these
    // same targets -- see midi_map.rs).
    let modbus = Arc::new(ModBus::new());

    // --- MIDI input: listens on any available port (e.g. macOS IAC Driver,
    // a MIDI keyboard, or an Ableton Push 2 used as a grid/knob controller) ---
    let midi_cutoff = Arc::clone(&cutoff);
    let midi_controller = Arc::clone(&controller);
    let midi_prism_cc = Arc::clone(&prism_cc);
    let midi_map_for_listener = Arc::clone(&midi_map);
    let midi_modbus = Arc::clone(&modbus);
    thread::spawn(move || {
        if let Err(e) = run_midi_listener(midi_cutoff, midi_controller, midi_prism_cc, midi_map_for_listener, midi_modbus) {
            eprintln!("MIDI listener stopped: {e}");
        }
    });

    // The mixing bus every app's audio processor renders into -- see
    // audio.rs. Every app gets one registered below, once, at startup;
    // it keeps running for the program's life regardless of which app
    // is on screen (see os.rs).
    let engine = audio::new_engine(Arc::clone(&master_volume));

    // Shared registry of tappable audio outputs -- lets Clouds
    // granulate another app's live signal instead of needing a
    // microphone input. See audio_bus.rs.
    let audio_bus = Arc::new(AudioBus::new());

    // Per-app channel levels for the Mixer app -- every audio-producing
    // app registers a channel here alongside its audio_bus one. See
    // mixer_bus.rs.
    let mixer_bus = Arc::new(MixerBus::new());

    // Owns the live output stream; Settings can request a different
    // device and Os::run's loop (main thread) applies it — see
    // audio_devices.rs for why that has to happen there.
    let (audio_host, device_state) = AudioHost::open_resilient(Arc::clone(&engine));
    let device_state = Arc::new(device_state);

    println!("Running. Send MIDI CC1 on any connected port to sweep the synth cutoff.");
    println!("Prism: CC{PRISM_TIME_CC}=Time CC{PRISM_REPEATS_CC}=Repeats CC{PRISM_SHAPE_CC}=Shape CC{PRISM_FILTER_CC}=Filter CC{PRISM_MIX_CC}=Mix CC{PRISM_SPACE_CC}=Space");
    println!("Menu: arrows to navigate, Enter to select. Esc: home.");
    println!("In an app: 1234/qwer/asdf/zxcv = grid, F1-F4 = top buttons, [ ] , . = knobs.");

    let manifests = manifest::discover(Path::new(APPS_DIR));
    println!("Found {} app manifest(s) in {APPS_DIR}", manifests.len());
    let registry = Registry::new(
        Arc::clone(&cutoff),
        Arc::clone(&device_state),
        Arc::clone(&sensitivity),
        Arc::clone(&nav_speed),
        Arc::clone(&modbus),
        Arc::clone(&audio_bus),
        Arc::clone(&master_volume),
        Arc::clone(&mixer_bus),
        Arc::clone(&prism_cc),
        Arc::clone(&show_cpu),
        Arc::clone(&midi_map),
        Arc::clone(&accent),
        Arc::clone(&background),
    );
    let mut apps = registry.build(&manifests);

    // Register every app's processor into the mix bus exactly once,
    // here — not on enter/exit (see os.rs). An app with no audio
    // (a settings-style screen) just doesn't add anything.
    for (_, app) in apps.iter_mut() {
        if let Some(processor) = app.audio_processor() {
            engine.add(processor);
        }
    }

    // Diagnostic escape hatch: render straight through the same
    // `engine` every real callback uses -- no cpal, no device, no
    // driver -- to a WAV file, and print level stats. Lets a "does
    // it actually sound bad" report be checked against the exact
    // production DSP output in isolation from anything the audio
    // driver/backend might also be doing to it. If `PORTAMAX_APP` is
    // also set, that app is driven with pad 0 held for the whole
    // render (same auto-start convention `PORTAMAX_APP` already has
    // in os.rs) instead of rendering silence/idle.
    if let Ok(path) = std::env::var("PORTAMAX_RENDER_WAV") {
        render_diagnostic_wav(&engine, &mut apps, &path);
        return Ok(());
    }

    // Same idea as the WAV escape hatch above, for the screen instead
    // of the speakers: dumps one rendered frame to a PNG with no
    // window, no GUI needed to actually look at a layout/color change.
    // `PORTAMAX_APP` picks which app's screen (unset: the launcher).
    if let Ok(path) = std::env::var("PORTAMAX_RENDER_PNG") {
        let app_name = std::env::var("PORTAMAX_APP").ok();
        let mut os = Os::new(apps, controller, audio_host, device_state, led_output::LedOutput::none());
        let mut fb = display::FrameBuffer::new();
        // PORTAMAX_SPLASH=<anything> dumps a startup logo screen
        // instead of an app/the launcher -- same "look at it without
        // a real window" escape hatch, for the boot screens that
        // aren't an app. The value picks which stage (see
        // `Os::draw_splash_for_diagnostics`): "mx1" for the second
        // logo, anything else (e.g. "1" or "szyk") for the first.
        if std::env::var("PORTAMAX_SPLASH").is_ok() {
            os.draw_splash_for_diagnostics(&mut fb);
        } else {
            os.draw_diagnostic_frame(app_name.as_deref(), &mut fb);
        }
        match write_png(&fb, &path) {
            Ok(()) => println!("render: wrote {path}"),
            Err(e) => eprintln!("render: couldn't write {path}: {e}"),
        }
        return Ok(());
    }

    // The OS owns the main thread from here — required on macOS, where the
    // window/event loop must run there. Audio and MIDI keep running on
    // their own threads/callbacks regardless of what's on screen. LED
    // output connects here too (not the MIDI input thread) since
    // sending is a plain blocking write, and `Os::run`'s loop is the
    // natural once-per-frame place to do it -- see led_output.rs.
    let leds = led_output::LedOutput::open_all();
    let os = Os::new(apps, controller, audio_host, device_state, leds);
    os.run();

    Ok(())
}

/// Renders `PORTAMAX_RENDER_SECONDS` (default 5) seconds of whatever
/// `engine` currently produces to a 16-bit stereo WAV at `path`,
/// printing peak/RMS/clip-count stats either way. If `PORTAMAX_APP`
/// names one of `apps`, that app is entered and driven with pad 0
/// held (a sustained note on anything that plays one) for the whole
/// render; otherwise this renders whatever the idle/default mix
/// produces with nothing touched at all.
/// Encodes a `FrameBuffer`'s packed `0x00RRGGBB` pixels (see
/// `display.rs`) as an 8-bit RGB PNG at `path`.
fn write_png(fb: &display::FrameBuffer, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, display::WIDTH as u32, display::HEIGHT as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;

    let mut data = Vec::with_capacity(display::WIDTH * display::HEIGHT * 3);
    for &px in fb.buffer() {
        data.push(((px >> 16) & 0xFF) as u8);
        data.push(((px >> 8) & 0xFF) as u8);
        data.push((px & 0xFF) as u8);
    }
    writer.write_image_data(&data)?;
    Ok(())
}

fn render_diagnostic_wav(engine: &audio::ActiveProcessor, apps: &mut [(String, Box<dyn App>)], path: &str) {
    let sample_rate = 48000.0;
    let channels = 2usize;
    let seconds: f32 = std::env::var("PORTAMAX_RENDER_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let frames = (seconds * sample_rate) as usize;
    let block = 512;

    let driven = std::env::var("PORTAMAX_APP").ok().and_then(|name| apps.iter().position(|(n, _)| n.eq_ignore_ascii_case(&name)));
    if let Some(i) = driven {
        apps[i].1.on_enter();
        // Apps with a real transport (Bloom/Madness/Sequencer/Nebula/
        // Tape) are driven by starting it, since their sound doesn't
        // come from the grid at all; apps played via the pad grid
        // (Cascade/Voltage/Plaits/Synth) are driven by holding pad 0
        // instead, below. Doing both is harmless either way.
        if apps[i].1.running() == Some(false) {
            apps[i].1.toggle_running();
        }
        println!("render: driving '{}' (started if it has a transport, pad 0 held throughout)", apps[i].0);
    } else {
        println!("render: no PORTAMAX_APP set -- rendering the idle/default mix with nothing touched");
    }
    let held_input = Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() };

    let spec = hound::WavSpec { channels: channels as u16, sample_rate: sample_rate as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = match hound::WavWriter::create(path, spec) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("render: couldn't create {path}: {e}");
            return;
        }
    };

    let mut buffer = vec![0.0f32; block * channels];
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut clip_count = 0u64;
    let mut total_samples = 0u64;

    // How long real hardware would actually give this callback before
    // it counts as late (an underrun/xrun -- the audio backend has to
    // glitch/repeat/skip, which is exactly what "computer garbling"
    // sounds like). Measuring wall-clock time per block here answers
    // "is the DSP itself simply too slow for 14 apps," completely
    // separate from anything about levels/clipping.
    let budget = Duration::from_secs_f32(block as f32 / sample_rate);
    let mut worst_block = Duration::ZERO;
    let mut over_budget_blocks = 0u64;
    let mut total_blocks = 0u64;

    let mut rendered = 0usize;
    while rendered < frames {
        let this_block = block.min(frames - rendered);
        if let Some(i) = driven {
            // `tick` runs once per UI frame in real use (~60Hz), not
            // once per audio block -- but calling it every block here
            // just means the "key" is (re-)confirmed held far more
            // often than a real controller would, which is harmless
            // for a steady, unchanging hold like this.
            apps[i].1.tick(&held_input);
        }
        let block_start = std::time::Instant::now();
        engine.process(&mut buffer[..this_block * channels], channels, sample_rate);
        let elapsed = block_start.elapsed();
        total_blocks += 1;
        worst_block = worst_block.max(elapsed);
        if elapsed > budget {
            over_budget_blocks += 1;
        }
        for &s in &buffer[..this_block * channels] {
            peak = peak.max(s.abs());
            sum_sq += (s as f64) * (s as f64);
            total_samples += 1;
            if s.abs() >= 0.999 {
                clip_count += 1;
            }
            let clamped = s.clamp(-1.0, 1.0);
            writer.write_sample((clamped * i16::MAX as f32) as i16).ok();
        }
        rendered += this_block;
    }
    writer.finalize().ok();

    let rms = (sum_sq / total_samples.max(1) as f64).sqrt();
    println!("render: wrote {seconds}s to {path}");
    println!("render: peak={peak:.4}  rms={rms:.4}  samples-at-or-past-clip={clip_count}/{total_samples} ({:.2}%)", clip_count as f64 / total_samples.max(1) as f64 * 100.0);
    println!(
        "render: block budget={:.2}ms  worst block={:.2}ms  blocks over budget={over_budget_blocks}/{total_blocks} ({:.2}%)",
        budget.as_secs_f64() * 1000.0,
        worst_block.as_secs_f64() * 1000.0,
        over_budget_blocks as f64 / total_blocks.max(1) as f64 * 100.0
    );
    if over_budget_blocks > 0 {
        println!("render: *** at least one block took longer than real hardware would allow -- this is what a real-time underrun/glitch/\"garbling\" looks like ***");
    }
}

fn handle_midi_message(
    message: &[u8],
    cutoff: &Arc<AtomicF32>,
    controller: &Arc<ControllerState>,
    prism_cc: &Arc<PrismCcTargets>,
    midi_map: &Arc<midi_map::MidiMap>,
    modbus: &Arc<ModBus>,
) {
    if message.len() < 2 {
        return;
    }
    let status = message[0] & 0xF0;
    let data1 = message[1];
    let data2 = message.get(2).copied().unwrap_or(0);

    match status {
        0x80 | 0x90 => {
            // Note On with velocity 0 is conventionally treated as Note Off too.
            let is_on = status == 0x90 && data2 > 0;
            if let Some(i) = controller::GRID_NOTES.iter().position(|&n| n == data1) {
                controller.grid[i].store(is_on, std::sync::atomic::Ordering::Relaxed);
            } else if let Some(i) = controller::TOP_NOTES.iter().position(|&n| n == data1) {
                if is_on {
                    controller.set_top(i);
                }
            } else if let Some(i) = fallback_grid_pad(data1) {
                // Not one of the Push 2 pads/top buttons `GRID_NOTES`/
                // `TOP_NOTES` were hand-tuned for -- still play it
                // rather than dropping it, so any other connected
                // keyboard (or a Push 2 pad outside that one picked
                // region) works as a musical keyboard too. Wraps the
                // whole 128-note MIDI range across the 16 pads, same
                // as the grid itself only ever has 16 notes' worth of
                // range -- two real notes 16 apart alias onto the same
                // pad, an honest limit of a 16-pad control surface,
                // not something worth a bigger redesign for.
                controller.grid[i].store(is_on, std::sync::atomic::Ordering::Relaxed);
            } else {
                eprintln!("midi: unmapped note {data1} (on={is_on})");
            }
        }
        0xB0 => {
            if data1 == 1 {
                // Mod wheel -- map 0-127 to a 100 Hz - 8000 Hz cutoff range, log scale
                let cc_value = data2 as f32 / 127.0;
                let hz = 100.0 * (80.0f32).powf(cc_value);
                cutoff.set(hz);
            } else if data1 == KNOB1_CC {
                controller.add_knob1_delta(decode_relative(data2));
            } else if data1 == KNOB2_CC {
                controller.add_knob2_delta(decode_relative(data2));
            } else if data1 == KNOB1_BUTTON_CC && data2 >= 64 {
                controller.set_knob1_press();
            } else if data1 == KNOB2_BUTTON_CC && data2 >= 64 {
                controller.set_knob2_press();
            } else if data1 == HOME_CC && data2 >= 64 {
                controller.set_home();
            } else if data1 == PRISM_TIME_CC {
                let v = data2 as f32 / 127.0;
                prism_cc.time.set(apps::prism::MIN_TIME + v * (apps::prism::MAX_TIME - apps::prism::MIN_TIME));
            } else if data1 == PRISM_REPEATS_CC {
                prism_cc.repeats.set((data2 as f32 / 127.0) * apps::prism::MAX_REPEATS);
            } else if data1 == PRISM_SHAPE_CC {
                prism_cc.shape.set(data2 as f32 / 127.0);
            } else if data1 == PRISM_FILTER_CC {
                prism_cc.filter.set(data2 as f32 / 127.0);
            } else if data1 == PRISM_MIX_CC {
                prism_cc.mix.set(data2 as f32 / 127.0);
            } else if data1 == PRISM_SPACE_CC {
                prism_cc.space.set(data2 as f32 / 127.0);
            } else {
                // Not one of the fixed control CCs above -- check the
                // user's own MIDI Learn mappings (see midi_map.rs)
                // before giving up on it.
                midi_map.observe_cc(data1, data2, modbus);
            }
        }
        _ => {}
    }
}

/// Connects to *every* available MIDI input port at once — an audio
/// interface's MIDI thru, a keyboard, and a Push 2 can all be plugged in
/// simultaneously, and this doesn't have to guess which one is "first".
/// Each connection is kept alive in `_connections` for as long as this
/// thread runs.
fn run_midi_listener(
    cutoff: Arc<AtomicF32>,
    controller: Arc<ControllerState>,
    prism_cc: Arc<PrismCcTargets>,
    midi_map: Arc<midi_map::MidiMap>,
    modbus: Arc<ModBus>,
) -> Result<(), Box<dyn std::error::Error>> {
    let probe = MidiInput::new("portamax-sim-probe")?;
    let port_count = probe.ports().len();
    if port_count == 0 {
        println!("No MIDI input ports found (set up the IAC Driver in Audio MIDI Setup on macOS).");
        return Ok(());
    }

    println!("MIDI ports found:");
    for (i, port) in probe.ports().iter().enumerate() {
        println!("  [{i}] {}", probe.port_name(port).unwrap_or_default());
    }

    let mut connections = Vec::new();
    for i in 0..port_count {
        let mut midi_in = MidiInput::new("portamax-sim")?;
        midi_in.ignore(Ignore::None);
        let port = match midi_in.ports().into_iter().nth(i) {
            Some(port) => port,
            None => continue, // port list changed since the probe; skip it
        };
        let name = midi_in.port_name(&port)?;

        let cutoff = Arc::clone(&cutoff);
        let controller = Arc::clone(&controller);
        let prism_cc = Arc::clone(&prism_cc);
        let midi_map = Arc::clone(&midi_map);
        let modbus = Arc::clone(&modbus);
        let conn = midi_in.connect(
            &port,
            "portamax-sim-in",
            move |_stamp, message, _| handle_midi_message(message, &cutoff, &controller, &prism_cc, &midi_map, &modbus),
            (),
        );
        match conn {
            Ok(conn) => {
                println!("Connected to MIDI input: {name}");
                connections.push(conn);
            }
            Err(e) => eprintln!("Couldn't connect to MIDI input '{name}': {e}"),
        }
    }

    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push 2's touch-sensitive encoders send Note On/Off on very low
    /// note numbers just from resting a finger on one -- those must
    /// never reach the fallback keyboard mapping, or every touch
    /// plays a note (see `MIN_FALLBACK_NOTE`'s doc comment).
    #[test]
    fn fallback_grid_pad_ignores_encoder_touch_range() {
        for note in 0..MIN_FALLBACK_NOTE {
            assert_eq!(fallback_grid_pad(note), None, "note {note} is below MIN_FALLBACK_NOTE and must not trigger a pad");
        }
    }

    /// Anything at or above a real 88-key piano's lowest note (A0,
    /// MIDI 21) must still play something, wrapped across the 16 pads.
    #[test]
    fn fallback_grid_pad_covers_the_real_keyboard_range() {
        assert_eq!(fallback_grid_pad(21), Some(21 % 16));
        assert_eq!(fallback_grid_pad(60), Some(60 % 16)); // middle C
        assert_eq!(fallback_grid_pad(127), Some(127 % 16));
    }
}
