# The Portamax synthesis platform

`src/synthesis/` is one engine that more than one app builds sounds with.
Oracle (patches a language model writes) and Atlas (curated presets with
macros and morphing) both run it. It is not a synth class. It is a
serializable graph of DSP blocks that gets compiled into a real-time engine.
A new sound is a JSON file, not new Rust code.

```
patch JSON ──migrate──▶ Patch ──compile (UI thread)──▶ Engine ──process (audio thread)
   (versioned)          (serde)   validate, allocate,      no locks, no allocation,
                                  cost estimate            no panicking paths
```

| Module | What it is |
|---|---|
| `patch.rs` | The format. It covers nodes, expressions, params with metadata (curve, unit, page, smoothing), macros, morph states A–D, settings, category/tags/author and the visual. It is versioned (`CURRENT_VERSION`), and `migrate` upgrades older files on load and refuses newer ones. |
| `blocks.rs` | The DSP node library. `SPECS` is the single source of truth: the compiler validates against it, and Oracle's AI prompt is generated from it. Adding a block makes it usable everywhere at once. `cost()` gives each block's measured price. |
| `expr.rs` | Small safe expressions, which are how nodes connect and modulate each other. |
| `engine.rs` | `compile` and `Engine::process`, the voice manager, macro application, param smoothing, the cost/polyphony budget and telemetry. |
| `evolve.rs` | Randomize, mutate and breed on normalized parameters. |

## Blocks

**Generators**
- `osc`: sine/tri/saw/square/pulse with PolyBLEP.
- `supersaw`.
- `wavetable`: tables analog/vocal/glass/metal/digital, with position, phase-mod and warp.
- `plaits`: the real Mutable Instruments engines via FFI.
- `sampler`: a WAV from `samples/`, with start/end/reverse/loop.
- `noise`: white/pink/brown.
- `modal`: an 8-mode resonator (strings, bars, bells, plates).
- `comb`: Karplus-Strong.

**Filters and shapers**
- `filter`: SVF lp/hp/bp/notch.
- `ladder`: 4-pole with per-stage saturation.
- `fold`.
- `drive`: soft/hard/tube/rectify.
- `crush`.

**Modulation**
- `adsr`, `ad`, `lfo`, `slew`, `sh`.
- `chaos`: logistic, Hénon and Lorenz maps.
- `clock`, `seq`, `euclid`.
- `follower`.

**Analysis**
- `pitch`: a monophonic pitch tracker (rising zero crossings with an envelope gate).
- `onset`.

**Effects**
- `delay`, `allpass`.
- `reverb`: an 8-line FDN.
- `shift`: a granular pitch shifter.
- `chorus`.
- `grain`: granular with freeze.

**Ported from Cardinal** (`blocks/ports.rs`)

These are ten modules from Cardinal, the open-source VCV Rack distribution,
that Portamax had no equivalent for. Mutable Instruments is left out
because its real code already runs here. Each port is written from the
published technique the module uses, not copied from its source. Nearly
all of Cardinal is GPL-3 and this project is MIT, so copying the code
would relicense it. Each doc comment says where the port differs from
the original.

| Block | After | What it is |
|---|---|---|
| `plateau` | Valley Plateau | Dattorro's figure-eight plate tank (the 1997 paper's delays and taps), mono |
| `spring` | Befaco / Surge Spring Reverb | Parker–Abel dispersive-allpass spring model: each bounce is the falling "drip" chirp |
| `phaser` | Surge XT Phaser | 4/6/8/12 first-order allpass stages, LFO and feedback |
| `freqshift` | Surge XT Frequency Shifter | Bode single-sideband shifter on Niemitalo's IIR Hilbert pair |
| `rotary` | Surge XT Rotary Speaker | Leslie horn and drum: crossover, Doppler, tremolo, spin-up inertia |
| `tape` | ChowDSP ChowTape | pre-emphasised tanh saturation, wow, flutter, head loss (no hysteresis solver) |
| `comp` | Bogaudio Pressor | soft-knee feed-forward compressor with sidechain |
| `kick` | Befaco Kickall | swept sine kick with drive |
| `walk` | Bogaudio Walk | bounded Brownian random walk with jump |
| `eq` | Bogaudio EQ | RBJ low shelf, bell and high shelf |

`plateau` holds about 1.4 s of delay lines (roughly 270 KB at 48 kHz),
so on the device use it in global scope, not per voice.

Each node's inputs are expressions. A node's id can be read by any later node,
and also by earlier ones, in which case it reads the previous sample. So generators interact
directly:
- FM is `"freq": "pitch * (1 + mod * index)"`.
- Ring mod is `a * b`.
- A modulator can be modulated by another modulator.
- A resonator can be excited by a noise burst.
- An onset detector can trigger an envelope.
- Chaos can move a filter.

Expression functions:
- Maths: `sin cos tan tanh exp log pow sqrt abs sign min max clamp floor ceil round fract wrap`.
- Interpolation and conversion: `mix smoothstep step mtof ftom db`.
- Shaping: `sat fold crush`.
- Wave shapes: `tri saw sqr`.
- `noise`.

Built-in values:
- Voice scope: `pitch note gate vel trig vtime vindex ampenv`.
- Global scope: `time sr in voices padnote padgate padtrig bpm beat playing`.
- Macros: `m1..m8`.
- Every param by its id.

## Patch format (v2)

```json
{
  "version": 2,
  "name": "Init", "category": "Keys", "tags": ["init"], "author": "Portamax",
  "description": "...",
  "kind": "instrument",            // or "effect" (reads `in`) / "generator"
  "voices": 8, "amp_env": true,
  "visual": "scope",               // or "spectrum" -- what Atlas shows
  "page_names": ["Oscillators", "Filter", "Motion", "Effects"],
  "params": [
    {"id": "cutoff", "name": "Cutoff", "min": 60, "max": 14000, "default": 1800,
     "curve": "exp", "unit": "Hz", "page": 2, "smooth": 0.02}
  ],
  "macros": [
    {"name": "COLOR", "targets": [{"param": "cutoff", "amount": 0.5}]}
  ],
  "voice":  [ {"id": "a", "type": "osc", "wave": "saw", "in": {"freq": "pitch"}} ],
  "voice_out": "a * 0.5",
  "global": [ {"id": "room", "type": "reverb", "in": {"in": "voices", "size": 0.8}} ],
  "out": {"left": "voices + room * 0.3", "right": "voices + room * 0.3"},
  "states": [ {"params": {}}, {"params": {"cutoff": 6000}} ],
  "settings": {"attack": 0.01, "release": 0.6, "root": 48, "scale": 2}
}
```

- **Params** are stored normalized (0..1) at run time and mapped through
  their curve: `lin`, `exp`, `int`, `toggle` or `choice` with `options`.
  Each param is smoothed with its own `smooth` time, 10 ms by default.
- **Macros** add `macro × amount` to a target's normalized position, so one
  macro can move many params by different amounts, including negative ones.
- **States** A–D (up to `MAX_STATES`) store parameter values. Atlas's MORPH
  interpolates through them in order. A param missing from a state keeps its
  default.
- **Settings** are the player-side built-ins a preset sets on load:
  `level voices glide transpose root scale attack decay sustain release
  velocity`. `scale` indexes `engine::SCALES`.
- **Versions.** Version 1 was everything written before versioning: a single
  `state`, no metadata. `migrate` turns it into v2 with that state as A. A
  file from a newer build is refused with a clear error rather than
  half-loaded.

## Real-time contract

- `compile` runs off the audio thread. It does all validation and
  allocation, loads samples, and returns every error phrased so a person or
  a model can fix the patch.
- The app hands the `Engine` to its processor through a `try_lock`ed slot.
  The processor crossfades from the old engine (2048 samples) and hands the
  old one back to be freed on the UI thread. The audio thread never blocks,
  allocates or frees. Oracle and Atlas share this pattern; copy it from
  either.
- `Engine::process` resets any node whose output goes non-finite and flags
  `unstable`, so a runaway feedback loop costs a click, not the device.

## CPU budget

`blocks::cost` gives each block's price in units of one plain oscillator
voice-sample, measured with the ignored test `block_costs`:

```
cargo test --bin portamax-sim block_costs -- --ignored --nocapture
```

Input expressions add a little on top. `Engine::cost_voice` and
`cost_global` sum these over the graph. `voices_within(budget)` returns how
many voices fit in a budget; Atlas caps polyphony with it, so an expensive
patch plays fewer notes instead of glitching. Atlas's default budget is 400
units, and the Deep page lowers it to model a slower target. The factory
presets cost between 4 and 27 global units plus 6 to 21 per voice.

Some measured points (desktop simulator):
- An oscillator is about 16 ns per sample.
- The FDN reverb is 14× an oscillator.
- The ladder is 1.2×. It was 12× until the rational `fast_tanh` replaced
  libm's `tanh`.

## Adding a block

1. Add a `BlockSpec` to `SPECS` (inputs with defaults, options, a one-line
   description; this text goes straight into Oracle's prompt).
2. Add a `Block` variant, its constructor in `Block::new`, `reset`, and its
   arm in `tick`. No allocation in `tick`.
3. Give it a `cost` (run `block_costs`), and if it holds a delay line,
   a `delay_seconds` entry.
4. Write a behaviour test proving what it claims, such as the
   resonance, the pitch or the decay, not just that it doesn't panic.

## Atlas presets

Factory presets are in `assets/atlas/presets/` and are compiled in. Saved
presets go to `saves/atlas/presets/`. Atlas maps a preset's macros by name
onto its seven fixed knobs:

CHARACTER · COLOR · MOTION · SPACE · SHAPE · ENERGY · TEXTURE

MORPH, the eighth knob, sweeps the states. A test
(`atlas::tests::every_factory_preset_compiles_plays_and_morphs`) checks that
every factory preset:
- parses at the current version and uses only those macro names;
- compiles;
- sounds when played;
- stays finite and bounded with every macro and MORPH at both extremes.

The ignored `preset_report` test prints each preset's level and cost for
balancing.

## Not done yet (from the meta-synth spec)

- **No graph editor on the device.** Atlas's Deep page shows the compiled
  graph with live activity, but patches are edited as JSON (or by Oracle).
- **No additive or spectral (FFT) resynthesis blocks yet.** Wavetable,
  modal and granular cover some of that ground.
- **No per-voice modulation matrix UI.** Modulation is written as
  expressions.
- **No NPU/AI features in Atlas.** Oracle is the AI front end to the same
  engine.
