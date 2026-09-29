# Portamax

Portamax is a self-contained groovebox/instrument simulator: 51 installed
apps (synths, effects, sequencers, recorders) sharing one real-time audio
engine, one modulation bus, and one control surface (4x4 pad grid, D-pad,
four knobs/encoders, F1-F4). It's a software simulation of a piece of
hardware that doesn't exist yet — an STM32N6-based device — built so the
architecture (buffer sizing, control-rate/audio-rate bridging, routing,
where an NPU inference call would sit in the signal chain) can be validated
entirely on a Mac before any hardware is built.

Several apps are ports of real, well-known DSP rather than approximations
of it — Mutable Instruments' Plaits, Clouds, Warps, and Beads all run their
actual C++ engines via FFI, not reimplementations. See
[docs/USER_MANUAL.md](docs/USER_MANUAL.md) for the full app list and what
each one actually does today.

## Quickstart

```sh
git clone --recurse-submodules https://github.com/Szyk-fi/SZYK.git portamax-sim
cd portamax-sim
cargo run
```

`--recurse-submodules` matters — `vendor/eurorack` (Mutable Instruments'
real DSP source) is a git submodule. If you already cloned without it:

```sh
git submodule update --init
```

`cargo run` launches the main simulator: an `embedded_graphics`/`minifb`
window standing in for the device's real screen, driven by keyboard, a
connected MIDI controller, or (macOS only) a PS5 DualSense controller. There's
also a Slint-based UI preview of the same underlying app logic:

```sh
cargo run --example slint_home_live
```

The first `cargo run` will take a while — it's building the whole DSP
engine plus several vendored C++ libraries via `cxx`. Subsequent builds are
incremental.

### Running the tests

```sh
cargo test
```

## Controls

- **Keyboard**: arrow keys navigate, `[`/`]` and `,`/`.` are the two
  encoders, `\` and `/` are their clicks, number/letter keys `1234 QWER
  ASDF ZXCV` are the 4x4 pad grid, `F1`-`F4` are the top row.
- **MIDI controller**: an Ableton Push 2 (or any class-compliant MIDI
  controller sending the same note/CC layout) drives the same pads/knobs —
  see `src/controller.rs` for the exact note map.
- **PS5 DualSense (macOS only)**: D-pad navigates and edits values, face
  buttons and shoulders map to select/back/F2/F3/Mixer/Home, right stick
  browses (left stick is intentionally unused — see `src/gamepad.rs` for
  why). Needs `GameController.framework`, which is macOS 11.3+ only; no
  gamepad support on other platforms yet.

Full navigation semantics (what each button does in which context) are in
[docs/USER_MANUAL.md](docs/USER_MANUAL.md#1-navigation-basics).

## Platform support

Developed and tested on macOS (Apple Silicon). The core audio engine
(`cpal`) and most DSP are cross-platform, but gamepad support is macOS-only
by design (`GameController.framework` has no equivalent elsewhere), and the
rest of the app hasn't been verified on Linux or Windows. If you try it on
another platform, expect keyboard-only control at best, and please report
what breaks.

## The Retro app and ROMs

One app, **Retro**, is a multi-console emulator front end (NES, SNES, Game
Boy/Color, a fixed set of classic arcade boards, and an in-progress Neo Geo
core). No ROMs, BIOS files, or copyrighted sample content ship with this
repo — `roms/` and `saves/` are entirely gitignored, and you supply your own
legally-owned dumps under `roms/<console>/`. See
[docs/USER_MANUAL.md](docs/USER_MANUAL.md) for the exact folder layout each
console expects.

**Current known limitation**: the Neo Geo core is real but incomplete — it
needs a user-supplied BIOS dump to make any progress past boot, and even
with one supplied, a real cartridge currently stalls before producing
visible video (documented in detail in `Console::NeoGeo`'s own doc comment
in [src/apps/retro.rs](src/apps/retro.rs)). Every other console in Retro
runs real games end-to-end.

## Known issues / what's still in progress

This is a testing build, not a finished product. Notable rough edges:

- Neo Geo emulation (see above) — boots, doesn't yet render.
- Portal's patch matrix doesn't persist patches across sessions yet.
- Some apps' bespoke visualizers haven't been ported into the connected
  Slint shell (`slint_home_live`) yet — they render generic parameter
  lists there instead of their own screen.

If you hit something that looks broken and isn't listed here, it probably
is a bug — please report it (see below).

## Reporting issues

Open an issue on this repo: <https://github.com/Szyk-fi/SZYK/issues>.
Include what you were doing, what you expected, and — if it's a crash —
whatever the terminal printed.

## License

MIT — see [LICENSE](LICENSE). This covers Portamax's own code only; bundled
third-party DSP sources (the `vendor/eurorack` submodule, vendored crates)
keep their own original licenses.
