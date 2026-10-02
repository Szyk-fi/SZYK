# Adding a new app

This is the exact, mechanical checklist for turning a new app's code into a
real, installed one. Read [CLAUDE.md](../CLAUDE.md) first if you haven't —
it covers the conventions this checklist assumes.

## Why this isn't drag-and-drop

`registry.rs` maps each app's manifest `id` to real Rust code through a
compile-time `HashMap` of constructor closures — there's no plugin system,
no dynamic loading, no `.zip`-and-drop mechanism. That's a deliberate
architectural choice (see `registry.rs`'s own module doc comment): a
`libloading`/WASM-style plugin runtime would be real, ongoing engineering
risk, and it wouldn't carry over to the actual target hardware this whole
simulator exists to validate (an STM32N6 board, which will load apps some
other way entirely — almost certainly not a dynamic loader). So every new
app needs a short, manual wiring step and a recompile. This checklist makes
that step as close to mechanical as it can be.

## What a tester's agent should hand back

One self-contained `.rs` file implementing the `App` trait (copy
[`src/apps/template.rs`](../src/apps/template.rs) as the starting point —
it's real, compiling, tested code, not pseudocode), plus:

- The app's chosen `id` (lowercase, `snake_case`, must not collide with an
  existing one — check `src/registry.rs`'s constructor map) and display
  `name`.
- A one-paragraph description of what it does and why (goes in the file's
  own module doc comment, and ideally in a message alongside the file).
- Confirmation it builds clean (`cargo build`, zero new warnings) and, if it
  makes sound, a test proving real audio comes out and stops when expected
  (see `template.rs`'s own test for the shape this should take).

Nothing else is required. Don't touch `registry.rs`, `apps/mod.rs`, or any
`apps/*.toml` file — that's the integration step below, done once the file
comes back.

## Integrating it (what you do when a tester sends a file back)

1. **Drop the file in.** Save it as `src/apps/<id>.rs`.
2. **Declare the module.** Add `pub mod <id>;` to `src/apps/mod.rs` (order
   doesn't strictly matter — the existing list isn't perfectly
   alphabetical — but keeping it roughly sorted helps readability).
3. **Write the manifest.** Create `apps/<id>/manifest.toml`:
   ```toml
   id = "<id>"
   name = "<Display Name>"
   ```
   (See any existing `apps/*/manifest.toml` for the exact shape — it's
   always just these two fields.)
4. **Register the constructor** in `src/registry.rs`:
   - Add the app's type to the big `use crate::apps::{...}` import list near
     the top of the file.
   - Add an entry to the `constructors` map, following whichever existing
     entry most closely matches what your new app's `new()` takes as
     arguments — `constructors.insert("synth".into(), ...)` is the simplest
     real example (a `Synth`-style app needing nothing shared with other
     apps); apps taking `Arc<...>` state shared with other apps (a global
     cutoff, the audio bus, etc.) look more like the `"plaits"` or `"warps"`
     entries just below it.
   - If the display-name lookup near the bottom of the file
     (`fn ... { let name = match id { ... } }`) is used anywhere your new
     app needs to show up in, add its `id => "Display Name"` arm there too
     — check whether this actually matters for your case; several newer
     apps read their name from the manifest instead and don't need this.
5. **Build and smoke-test.**
   ```sh
   cargo build
   cargo test
   cargo run
   ```
   Confirm the new app appears in the launcher, its screen draws, its
   controls respond, and (if it makes sound) audio actually comes out.
6. **Commit.** One commit per app is fine; mention the tester's name in the
   commit message if you want the credit trail.

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
