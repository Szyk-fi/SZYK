# Portamax — context for an agent writing a new app

> **Two assistants work on this repo.** Read [AGENTS.md](AGENTS.md) first and follow it: ownership, locks and the shared work log live there.

Read this before writing any code. If you're building a new app to hand
back to the project owner, your target checklist is
[docs/ADDING_AN_APP.md](docs/ADDING_AN_APP.md) — read that too. Your
starting point is [`src/apps/template.rs`](src/apps/template.rs): a real,
compiling, tested, minimal app. Copy it, don't start from a blank file.

## What this project actually is

Portamax is a software simulation of a hardware groovebox/instrument that
doesn't exist yet (an STM32N6-based device). 58 installed apps — synths,
effects, sequencers, recorders — share one real-time audio engine, one
modulation bus, and one control surface (4x4 pad grid, D-pad, two
knobs/encoders, four top buttons F1-F4). See
[docs/USER_MANUAL.md](docs/USER_MANUAL.md) for what every existing app does.

**The one rule that matters most: no fakes.** Several apps here are real
ports of well-known hardware/software DSP — Mutable Instruments' Plaits,
Clouds, Warps, and Beads run their actual C++ engines via `cxx` FFI, not
reimplementations or approximations. When you build a new app, its DSP
should be genuinely correct for what it claims to be (a real filter
topology, a real oscillator waveform, a real sequencer clock division) —
not a plausible-looking stand-in. If you're not sure something is right,
say so in a comment rather than presenting a guess as settled.

## The `App` trait (what a new app must implement)

Defined in [`src/app.rs`](src/app.rs). Only two methods are required:

```rust
fn tick(&mut self, input: &Input);       // called once per frame — read controls, update state
fn draw(&mut self, fb: &mut FrameBuffer); // the embedded_graphics screen main.rs actually shows
```

Everything else has a sensible default (see `template.rs` for the ones
worth overriding on a first app):

- `audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>>` — return
  `None` if the app makes no sound at all (a pure editor/utility app).
  Called once, lazily, the first time it's actually needed.
- `slint_rows(&self) -> Vec<(String, String, bool)>` — real menu rows
  `(name, value, is_group)` for the connected Slint UI
  (`examples/slint_home_live.rs`). Every app should have this even if it
  never gets its own bespoke Slint screen — it's what the generic
  `ParamListColumn` shows.
- `running(&self) -> Option<bool>` / `toggle_running(&mut self)` — only if
  your app has a startable/stoppable transport concept; this is what the
  global F3 button reflects and controls.
- `background_tick(&mut self)` — called once per frame for *every*
  installed app, not just the active one. The one real use so far is CV Out
  sending MIDI CC out (`src/apps/cv_out.rs`) — real blocking I/O that must
  never run on the audio thread. If your app needs to do something
  continuous that isn't audio and can't wait for the app to be on-screen,
  this is where it goes, not `tick`.

## Real-time audio rules (non-negotiable)

`AudioProcessor::process` (`src/audio.rs`) runs on the actual audio
callback thread with a real deadline — missing it is audible glitching, not
a logged error. Inside `process`:

- **Never block.** No `Mutex::lock()` that could contend with the UI
  thread for long, no I/O, no allocation you can avoid. Share state with
  the UI side via `Arc<AtomicF32>`/`Arc<AtomicBool>`/etc. (see
  `crate::util::AtomicF32`) for single values; use a `Mutex` only for
  something richer, and keep the critical section tiny (see
  `SynthApp`'s `held: Arc<Mutex<[bool; 16]>>` for the pattern).
- `buffer` is interleaved by `channels` — walk it with
  `buffer.chunks_mut(channels)`, one frame at a time. This is the standard
  pattern every app in this codebase uses.
- If your app's failure mode is "produces silence" rather than "panics,"
  prefer that — `audio.rs` disables a processor after too many consecutive
  panics in a row, but a silent app is still a broken one from a tester's
  perspective.

## Talking to other apps

Nothing is hardwired between apps. Two real mechanisms exist:

- **Audio** (`src/audio_bus.rs`): an app that produces audio others might
  want to process calls `audio_bus.register("Name")` once, at construction,
  getting back a buffer to overwrite every `process()` call. A consumer
  (an effect) exposes a **Source** row letting the user pick which
  published output to read, via `audio_bus.get(idx)`. See `WarpsApp`
  (`src/apps/warps.rs`) for a real, working example of both directions —
  Warps publishes its own output *and* reads two other apps' outputs as
  carrier/modulator. Selecting a source never auto-starts anything;
  transports stay explicit.
- **Modulation** (`src/modbus.rs`): parallel to the audio bus but carries a
  single control-rate number instead of a whole audio block — this is what
  Portal's patch matrix and MIDI Learn's CC mappings both ride on.

Don't reach into another app's struct directly. If your new app needs
another app's output, that's what a Source row is for.

## Conventions worth matching

- **Comments explain *why*, not *what*.** A well-named function or
  variable already says what it does; a comment earns its place by
  recording a non-obvious constraint, a real hardware fact being modeled,
  or the reasoning behind a specific number (see nearly every existing
  app's doc comments for the house style — verbose on rationale, silent on
  the obvious).
- **Every app picks its own color palette**, not a shared device-wide
  theme — see `template.rs`'s `BG`/`TITLE`/`DIM` constants for the pattern.
- **Test real behavior, not implementation details.** A synth-style app's
  test should prove "holding a pad produces real audio, releasing it goes
  quiet" (see `template.rs`'s own test) — not just "the struct constructs
  without panicking."
- **Real ROM/BIOS/sample content is never committed.** If your app touches
  the Retro console emulator or needs sample audio, see the README's
  section on that — user-supplied only, always gitignored.

## What you don't need to touch

Don't edit `src/registry.rs`, `src/apps/mod.rs`, or create an
`apps/<id>/manifest.toml` — that's the project owner's integration step
(see [docs/ADDING_AN_APP.md](docs/ADDING_AN_APP.md)), done once your file
is handed back. Your job is one self-contained `.rs` file plus a short
description of what it does.
