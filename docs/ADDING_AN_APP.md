# Adding a new app

This is the exact, mechanical checklist for turning a new app's code into a
real, installed one. Read [CLAUDE.md](../CLAUDE.md) first if you haven't —
it covers the conventions this checklist assumes.

## How an app gets installed

Dropping a file in is the whole job. Nothing central is edited:

- `build.rs` scans `src/apps/` on every build. Each `<id>.rs` (or
  `<id>/mod.rs`) becomes a module, and every module that exports
  `pub fn create(&AppContext, &str) -> Box<dyn App>` is added to the
  generated `FACTORIES` table. A file pulled into another module with
  `#[path = ...]` or `include!` is that module's private part, not an app.
- At startup `manifest::discover` reads every `apps/<id>/manifest.toml`
  (and the SD card's `apps/` folder). `Registry` pairs each manifest with a
  factory by `module` (default: the id). A manifest with no code is skipped
  with a warning, so nothing breaks the menu.
- Everything else comes from the manifest and the shared buses, so an app
  gets it for free:

| You write in the manifest | The app gets |
|---|---|
| `category`, `name`, `description` | a place in the launcher |
| `audio_outputs = ["Name"]` | a mixer channel, and a pick in every effect's Source row |
| `notes_in = true` | it appears in every sequencer's "Plays" picker and receives their notes like a MIDI keyboard, on screen or not |
| `note_outputs = ["Name"]` | its notes can be routed to any instrument (see `NoteRoute` in `src/note_bus.rs`) |
| `mod_inputs = [...]` | its knobs are patchable from every modulation source before the app has ever been opened |

## What a tester's agent should hand back

One self-contained `src/apps/<id>.rs` (copy
[`src/apps/template.rs`](../src/apps/template.rs): real, compiling, tested
code, not pseudocode) and its `apps/<id>/manifest.toml`, plus:

- The app's `id` (lowercase `snake_case`, unique) and display `name`.
- A one-paragraph description of what it does, in the file's module doc
  comment.
- Confirmation `cargo build` is clean (no new warnings) and, if it makes
  sound, a test proving audio comes out and stops when expected (see
  `template.rs`).

## Steps

1. **Add `src/apps/<id>.rs`** with a `pub fn create` at the bottom:
   ```rust
   pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
       Box::new(MyApp::new(ctx.try_get()))   // ask ctx for what you need
   }
   ```
   `ctx.get::<T>()` / `ctx.try_get::<T>()` hand you the shared services
   (`NoteBus`, `AudioBus`, `ModBus`, ...). `ctx.named("sensitivity")` gives
   the shared Settings values. Look at `turing_machine.rs` for a real one.
2. **Add `apps/<id>/manifest.toml`:**
   ```toml
   id = "<id>"
   name = "<Display Name>"
   category = "instrument"   # instrument, effect, sequencer, library, utility, game, kids, ai
   description = "One sentence."
   notes_in = true           # if other apps can play it
   note_outputs = ["<Name>"] # if it sends notes
   audio_outputs = ["<Name>"] # if it publishes audio
   ```
3. **Fill in `mod_inputs`.** If the app registers modulation inputs
   (`modbus.register("<Name>: <Param>")`), run
   `tools/sync_manifests.sh` once. It rewrites the list from what the code registers, and a plain
   `cargo test` fails if the two drift apart. Name inputs `"<App>: <Param>"`.
4. **Build and test.**
   ```sh
   cargo build
   cargo test --bin portamax-sim
   cargo run
   ```
   The `every_installed_app_*` tests in `src/registry.rs` build every
   manifest through the real registry, so a missing `create`, a manifest
   with no code, a duplicate id or a note output that can't be routed fails
   there, with the app's name.
5. **Commit.** One commit per app is fine.

Not drag-and-drop in one respect: the code is compiled in, so a new `.rs`
file needs a rebuild. A data-only instrument (an Atlas patch) needs no code
or rebuild: a folder with a manifest (`module = "atlas"`, `data = "..."`).
See `a_cartridge_folder_is_a_new_playable_instrument`.

## Common ways a handed-back app won't build cleanly

- **Reaching into another app's internals directly** instead of through
  `AudioBus` (see `CLAUDE.md`'s "Talking to other apps" section) — a sign
  the app needs a `Source` row instead.
- **Blocking I/O or `Mutex::lock()` contention inside `AudioProcessor::
  process`** — anything that can stall there is an audible glitch, not just
  a slow path. See `CvOutApp` (`src/apps/cv_out.rs`) for how blocking work
  (sending MIDI CC) gets moved to `background_tick` instead.
- **New Cargo dependencies.** If the app needs a crate not already in
  `Cargo.toml`, add it there and note it in the integration commit — check
  it doesn't pull in something that breaks the existing
  `[target.'cfg(target_os = "macos")'.dependencies]` split (see the
  README's "Platform support" section) before merging.

## Play view

Playable apps open on the shared play view. See [PLAY_KIT.md](PLAY_KIT.md) for
the structure and the steps to give a new app one.

## Building a synth on the shared engine

If your app makes sounds from oscillators, filters and effects, consider
not writing the DSP yourself. The synthesis platform (`src/synthesis/`, see
[SYNTH_PLATFORM.md](SYNTH_PLATFORM.md)) compiles a JSON patch into a
real-time engine with:

- voices, macros, morph states and parameter smoothing;
- a CPU budget;
- telemetry.

Atlas (`src/apps/atlas.rs`) is the smallest complete front end to it, and its
engine handover (pending/retired slots plus a crossfade) is the pattern to
copy. Often a new "synth" is just a new preset in
`assets/atlas/presets/` rather than a new app.

## Wrapping a Mutable Instruments module (or other C++ DSP)

The eurorack repository is vendored in `vendor/eurorack`. To run another
of its modules:

1. Write `vendor/bridge/<name>_bridge.cc`: a small `extern "C"` wrapper
   that sets the module's patch and performance structs the way its
   firmware does (read `<module>/<module>.cc` and its `cv_scaler`/
   `cv_reader`) and calls its `Process`. No DSP of your own.
2. Write `vendor/bridge/<name>.sources`: the `.cc` files it needs, one
   per line, relative to `vendor/eurorack`. The module's own
   `test/makefile` lists them. `build.rs` compiles every `.sources` list
   it finds into `lib<name>_bridge.a`, so there's nothing to register.
3. Write the app in `src/apps/<name>.rs` with `src/apps/mi_kit.rs`:
   describe the panel as `Spec`s, implement `Module`, and wrap it in
   `MiApp`. That gives you the play view, the menu, pads as keys, note-bus
   input, mod inputs for every knob, and `RateBridge` for running a
   fixed-rate engine at the device's rate. `rings.rs` is the smallest
   voice, `marbles.rs` a note source, `tides.rs` a modulation source.
4. Check the license: the STM32 modules are MIT; the AVR ones (Grids,
   Shruthi...) are GPL and can't be linked into Portamax.

## A full-screen app (the Kids apps)

An app that draws its whole screen itself returns `true` from
`wants_fullscreen` and hands the same picture to the Slint GUI from
`slint_extra`:

```rust
fn slint_extra(&mut self) -> SlintExtra {
    let mut fb = FrameBuffer::new();
    self.draw(&mut fb);
    kids_kit::screen_extra(&fb)
}
```

The GUI then shows it edge to edge, with no menu column and no chrome,
so the device and the GUI show one design. `src/apps/kids_kit.rs` also has
a small sound engine (`Sound`: tuned percussion, harp, organ, drums, a
step clock that calls your `Song` on the audio thread, and an `Extra`
hook for your own DSP) and drawing helpers (big text, stars, rounded
boxes). `rainbow_bells.rs` is the smallest app built on it. Give the
manifest `category = "kids"` to file it under the launcher's Kids section.
