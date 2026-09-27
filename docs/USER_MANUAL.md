# Portamax User Manual

Portamax is a self-contained groovebox/instrument simulator: 51 installed
apps sharing one audio engine, one modulation bus, and one 4x4 pad grid +
D-pad + four knobs/encoders + F1-F4 control surface. This manual covers
what's actually implemented today, not a roadmap.

Run it with:

```sh
cargo run
```

or, for the live Slint UI preview used during development:

```sh
cargo run --example slint_home_live
```

## 1. Navigation basics

- **D-pad up/down** selects a parameter row; **left/right** changes its value.
- **R1** performs the selected row's action (load a scene, trigger a menu item, etc).
- **F1** goes Home (opens Settings from Home, if installed).
- **F2** is contextual: Sequencer's pad/step mode, or Pad Lock/Unlock for
  instruments that use the pad grid for notes (keeps the instrument
  listening to pads while another screen is open).
- **F3** always shows and performs the current app's next transport action
  (Record/Stop/Play, etc.) — never a generic "play" button.
- **F4** opens Mixer, when installed.
- The 4x4 pad grid means different things per app (notes, sample slices,
  track select, mute/solo...) — each app's own section below says what.

The launcher groups installed apps into **Instruments, Effects, Sequencing,
Library,** and **Utilities**. Up/down selects an app, left/right changes
category, R1 opens it. F2 cycles categories, F3 opens Recent (last 12 apps
opened this session, not persisted across restarts), F4 opens Mixer. A green
dot marks an app with an active transport.

## 2. How routing works

Nothing is hardwired. Every app that produces audio publishes it on a shared
bus under its own name; every app that accepts input (an effect, a recorder,
a mixer channel) has a **Source** row where you pick which published output
to listen to. Selecting a source does not start anything — transports and
hardware input both stay explicit. A source is tapped *before* its own Mixer
fader, so if you only want to hear an effect's processed output, turn the
dry app's Mixer channel down.

For an external microphone or interface: pick it in **Settings → Input**
first, then select **Hardware input** as the receiving app's Source. Nothing
opens the input device until you do that.

Portal is the dedicated patch-bay app (see §5) — open a destination app once
to register its parameters as modulation targets, then wire it up in Portal.

## 3. Instruments

| App | What it does |
|---|---|
| **Plaits** | The real Mutable Instruments Plaits voice (ported DSP, not an approximation) — the same 16-model macro-oscillator as the Eurorack module. |
| **Voltage** | Classic 2-oscillator subtractive synth (Saw/Square/Triangle/Sine), filter, envelope. |
| **Cascade** | 6-operator FM synth in the spirit of the DX7 — real FM synthesis, not a clone of Yamaha's ROM. Loads real `.syx` presets. |
| **Synth** | The simplest instrument: 4x4 grid as a 16-note held keyboard, two knobs for cutoff/volume. |
| **Madness** | Independent per-position phase drift warping a polygon into a generative voice (started life as "Bloom", renamed once the mechanic diverged). |
| **Nebula** | A small real 2D gravity simulation — particles drift and get captured into orbits, each capture fires a note. |
| **Queen of Pentacles** | CV/gate generator driven by real 1D chaotic maps (logistic map and two relatives), not a disguised sequencer. |
| **StarLab** | Strymon StarLab-inspired: a Karplus-Strong string voice sharing a comb/allpass reverb tank with the effect side. |
| **Orbit / Swarm / Mutant / Constellation / Dream** | Five Collection-engine instruments — see §7. |

## 4. Effects

All effects tap another app's live audio via the shared bus (the **Source**
row) rather than generating their own signal.

| App | What it does |
|---|---|
| **Warps** | Mutable Instruments Warps clone: cross-modulates two live app outputs (carrier/modulator). |
| **Beads** | Mutable Instruments Beads clone: granular texture synth built on a live circular capture buffer. |
| **Clouds** | The real Mutable Instruments Clouds granular processor (ported DSP). |
| **Nautilus** | Qu-Bit Nautilus-inspired 8-line codependent stereo delay network. |
| **Black Hole** | Erica Synths Black Hole-inspired 24-algorithm glitch/texture multi-effect. |
| **Magnito** | Magnetic-tape saturation/hysteresis processor. |
| **Prism** | Multi-effect in the spirit of Hologram Microcosm. |
| **Singularity** | Four original, deliberately alien-sounding processes (not modeled on any real pedal). |
| **Rainmaker** | Intellijel/Cylonix Rainmaker-inspired 16-tap pitched rhythmic delay. |
| **Natural Gate** | Rabid Elephant Natural Gate clone: dual-channel zero-bleed low-pass gate. |
| **Tonestack** | Guitar amp-sim / effects-chain processor. |
| **Vector Filter** | 3D "vector" filter interface — drag a projected cube face: X = cutoff (log, 40–16kHz), Y = resonance, Z-strip = drive. Low/band/high-pass share one control surface. |
| **Fracture / Ghosts / Tape Machine / Portal** | Collection-engine effects — see §7. |

## 5. Sequencing & modulation

| App | What it does |
|---|---|
| **Bloom** | Generative circular sequencer built from two groups of the same element ("dots"). |
| **Pam's** | Clone of the core of Pamela's Pro Workout — multi-channel clock/gate generator with logic combinators between channels. |
| **Turing Machine** | Music Thing Modular Turing Machine clone: clocked 16-bit shift register, "Locks" sets random-vs-repeat. |
| **Sequencer** | Multi-track step sequencer in the spirit of Sugar Bytes DrumComputer. |
| **CV Out** | 32 independent CV outputs sent as MIDI CC to an external MIDI-to-CV box. |
| **MIDI Learn** | Browse/add/remove CC → modulation-target mappings. |
| **Portal** | Eight-cable patch matrix: any registered source (LFOs, clocked random, audio buses) to any registered destination (Portal's own sends, or another app's modulation parameter). Direct monitor is off by default. Patches last for the session only — no patch-file persistence yet. |

## 6. Recording & library

| App | What it does |
|---|---|
| **Tape** | 4-track looper/recorder — records any other app's live output. |
| **Sample Drum** | Erica Synths Sample Drum clone: dual-channel sample player/slicer. |
| **Reference / Field / Sample Hunter / Studio / Vinyl / Practice / Radio / Memories** | Collection-engine recording & library apps — see §7. |

## 7. The Collection apps (19 apps, one shared engine)

Nineteen apps share one `CollectionApp` engine, each with its own DSP
behavior selected by a `Kind`. All are real audio-clock-driven engines, not
skinned duplicates.

| App | Implemented behavior |
|---|---|
| Orbit | Audio-clock polyrhythmic sequencer; pads enable trigger points; ratio, pitch, gate, probability, swing. |
| Fracture | Real rolling-input slices with repeats, reverse, semitone pitch, scatter, dry/wet. |
| Ghosts | Four delayed generations of the input with geometric decay, drift, filtering, reverse reading. |
| Swarm | Up to 16 oscillators; cohesion/spread/mutation change detuning. |
| Mutant | Harmonic-parent blending, weighted partials, seeded patch mutation (Breed changes real synthesis coefficients). |
| Constellation | Chord degrees, inversions, octave spread, glide, tension drive real oscillator pitches. |
| Tape Machine | Four delay heads, variable timing/speed, bounded feedback, wow/flutter, saturation. |
| Dream | Audio-clock Calm/Tension/Chaos/Release state sequencing. |
| Portal | (see §5) |
| Reference | Stereo audio-file playback, seeking, gain, peak normalization, repeat. |
| Field | Selected-input recording: pre-roll, gain, limiter, low cut, marker, WAV export. |
| Sample Hunter | Input capture, threshold gate, trim bounds, normalized export into the shared library. |
| Studio | Eight independent take buffers, pan, mute/solo, overdub, stereo mix export. |
| Scope | Real selected-input waveform/FFT — ten display modes (Waveform, Spectral Terrain, Interference Loom, Prismatic Shards, Harmonic Orrery, Phase Portrait, Spectral Crown, Spectrogram, Contour Field, Wave Ribbons). No audible output. |
| Vinyl | Audio-library browsing, variable-speed playback, repeat, auto-next. No invented metadata/artwork. |
| Practice | A–B loop with independent speed/pitch (overlapping grains) plus metronome. |
| Radio | User-configured HTTP(S) WAV streaming, bounded queue, resample, gain/limiter. MP3/AAC stations aren't supported — WAV only. |
| Master | A/B bus comparison, RMS-matched loudness, bass/presence, drive, output ceiling. |
| Memories | Shared recording-library playback plus per-file favorite/tag sidecars. |

Common recording pad layout: **1** Record/Stop, **2** Play/Stop, **3** Save
WAV, **4** Marker; Studio uses pads 5–12 for track select.

**Instruments** (Orbit/Swarm/Mutant/Constellation/Dream) share eight voice
choices (sine, glass FM, reed, plucked string, drawbar organ, round bass,
live-input wavetable, imported sample) and six arp/timing combos (Up, Down,
Bounce, Random, Random Walk, Chords × Straight, Offbeat, Euclidean 5/8,
Sparse, Bursts). Drop a mono/stereo file into `media/instruments/` (or any
media subfolder), **Rescan instruments**, then **Sample instrument** to use
your own sound — one-shot, pitched from C4, not a multisample instrument.

Every Collection app has four **Recall scene** slots (routes/transport/files
are untouched by recall — only parameters, so recall can change levels or
mutes).

**Scale system**: 16 scale families from the Berklee PULSE reference
(chromatic, major/natural/harmonic minor, the 5 modes, major/minor
pentatonic + blues, whole tone, both diminished sequences), shared by every
scale-capable app.

## 8. Retro — multi-console emulation

Retro is a real, multi-console platform, not one emulator: **NES, SNES,
Arcade, Game Boy/Game Boy Color,** and **Neo Geo**, selected from a
`Console` row so switching consoles doesn't mean diving into a submenu.
Every core is a real, vendored emulator statically linked into the binary
(see `src/apps/retro.rs`'s own module doc for the full reasoning) — nothing
here is simulated or faked.

ROMs are user-supplied and never committed to this repo (see `.gitignore`).
Drop them in:

```
roms/nes/        (.nes)
roms/snes/       (.sfc, .smc, .swc, .fig)
roms/arcade/     (one subfolder per game, named like the ROM set — e.g. roms/arcade/pacman/)
roms/gb/         (.gb, .gbc)
roms/neogeo/     (one subfolder per cartridge, containing its P1/P2/M1/S1/C1-C8 ROM files)
```

Save states, per console/game/slot, land in the matching `saves/` folder.

**Neo Geo is a special case, worth reading before you judge it harshly**: no
complete Neo Geo emulator exists anywhere in the Rust ecosystem to vendor
(checked; the only near-miss depends on the same dynamic-core-loading model
this project can't use on real hardware). Its 68000 and Z80 CPU cores are
real vendored crates, but the memory map, video (fix/text layer *and*
sprites — tile decode, position, flip, shrink), and the 68k↔Z80 sound
handshake are hand-written against public hardware documentation instead of
borrowed, and are still missing: all sound (the YM2610 chip), and most real
cartridges' protection-chip decryption (Metal Slug 3's "PVC" chip among
them) — so a protected cartridge will run real code for well under a second
before hitting code this core can't decrypt yet, and cleanly report
"emulation error, reload the ROM" the same way any other core's real crash
does. This is documented in detail, including exactly what's verified
against real cartridge bytes vs. still unverified, in `src/apps/neogeo_core.rs`'s
own module doc comment.

Save states aren't supported for Neo Geo yet.

## 9. PS5 controller support

A real DualSense (or compatible) controller works out of the box on macOS
via Apple's `GameController.framework` — D-pad navigates, face buttons map
to pad actions, sticks drive knobs, shoulder triggers are F2/F3, Menu is
Home. It needs a real window focused (the actual app, not a bare CLI probe)
to be detected — that's an Apple platform requirement, not a bug.

## 10. Known limits (read before filing something as broken)

- **Nothing runs until you ask it to.** Installing an app's manifest only
  creates a dormant factory. No DSP, FFT, recording buffer, media scan, or
  network request happens just because an app appears in the launcher.
- Captures are bounded: recording apps cap at 384,000 frames per track (8s
  @ 48kHz); local file playback caps at 5,760,000 frames (120s @ 48kHz).
  Long-form streaming playback isn't implemented.
- Radio streams uncompressed PCM/float WAV only — no MP3/AAC stations.
- Practice's time-stretch is a lightweight granular technique, not a
  transparent commercial stretch engine.
- Studio exports a stereo mix only; there's no persistent multitrack session
  format yet.
- Patches made in Portal last for the session — no save/load for routing yet.
- This is a desktop simulator. It cannot prove real STM32N6/H7 CPU, RAM, or
  audio-driver budgets — only that the software behavior itself is correct.
  Every module doc comment that makes a hardware-feasibility claim says so
  explicitly, including where it's still unverified.

## 11. Architecture, if you're going to touch the code

- One shared `AudioBus` carries every app's published output; one shared
  modulation bus carries registered parameter targets. Apps never look each
  other up by name — everything is dependency-injected. See
  `docs/app-independence.md`.
- Every real DSP/emulator core in this project is either a genuine, vendored
  third-party implementation (Mutable Instruments' own C++ via FFI,
  `tetanes-core`, `super-sabicom`, `phosphor-machines`, `rboy`, `m68k`,
  `z80`) or, where nothing existed to vendor (Neo Geo's system-level
  hardware), hand-written against public documentation and clearly flagged
  as such. Nothing is a stub pretending to be a real implementation.
- Run `cargo test --bin portamax-sim` before committing anything — the
  suite is large (400+ tests) and covers real behavior, including
  hardware-derived timing and byte-for-byte ROM decode checks, not just
  "doesn't panic."
