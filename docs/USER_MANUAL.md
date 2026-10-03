# Portamax User Manual

Portamax is a self-contained groovebox/instrument simulator: 56 installed
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

### 1.1 The play view

Every instrument, effect, sequencer and recorder opens on its **play view**:
the parameter list is replaced by a play column (four dials, the 16 pads
labelled for what they do right now, the stick and both hand sensors), next
to the app's own panel. The layout is the same in every app:

| Control | On the play view |
|---|---|
| **R1** | Toggle the full menu — everything is still there |
| **F2** | Cycle pad layers: the app's own (notes, steps, slices, gates...) first, then **Controls**, **Moments**, and **Throws** on effects |
| **L1 (hold)** | Peek at the Controls layer |
| **Knob 1 / 2** | Turn the hero pair (badged 1 and 2); knob 1 press = next pair, knob 2 press = reset the pair |
| **D-pad up/down** | Step the app's main choice (engine, preset, algorithm, octave, track, scene...) |
| **Stick, hands, mod wheel, aftertouch** | Push the routed controls away from their knob setting; letting go returns exactly to it; stick click keeps the pushed sound |
| **Controls layer** | 16 parameters on the pads; tap one, knob 2 turns it |
| **Moments layer** | Tap = recall a whole sound, hold 0.6 s = store (saved to `saves/<app>/moments.json`) |
| **Throws layer** | Hold a pad to push a control somewhere (feedback up, freeze, octave...); release springs back |

On a play-view app a MIDI keyboard plays the app's pads by pitch where the
pads are pitched. Utilities (Settings, Mixer, MIDI Learn, CV Out, Portal,
Scope, Analyzer, Visualizer, Retro, Controller) keep their menus.

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

### 2.1 Patching modulation

Modulation works the same way. Every app lists the parameters it accepts
modulation on (its *inputs*), and every installed app's inputs are
available from the moment Portamax starts — you don't have to open an app
first. Any modulation source (Turing Machine, Pam's, Queen of Pentacles,
Natural Gate's envelope out, Nautilus's sonar, Portal, MIDI Learn) picks a
destination in two steps:

1. **… App** — turn knob 2 to choose the app (or None). Landing on an app
   selects its first input.
2. **… Input** — the row right below it; knob 2 now walks only that app's
   inputs.

Each app's level fader also shows up as an input, under **Mixer**.
Patching into an app you haven't opened yet wakes it up so it can hear the
modulation. Portal (see §5) is the dedicated patch bay for LFOs and audio
followers; its **Destination** / **Destination input** rows work the same
way.

## 3. Instruments

| App | What it does |
|---|---|
| **Plaits** | The real Mutable Instruments Plaits voice (ported DSP, not an approximation) — all 24 engines of the Eurorack module's 1.2 firmware, with a play view built around the whole device (see §3.1). |
| **Voltage** | Classic 2-oscillator subtractive synth (Saw/Square/Triangle/Sine), filter, envelopes, mono/glide, chorus, delay and reverb. 100 preset slots, 20 synthwave factory presets. |
| **Cascade** | 6-operator FM synth in the spirit of the DX7 — real FM synthesis, not a clone of Yamaha's ROM. Loads real `.syx` presets. |
| **Synth** | The simplest instrument: 4x4 grid as a 16-note held keyboard, two knobs for cutoff/volume. |
| **Madness** | Independent per-position phase drift warping a polygon into a generative voice (started life as "Bloom", renamed once the mechanic diverged). |
| **Nebula** | A small real 2D gravity simulation — particles drift and get captured into orbits, each capture fires a note. |
| **Queen of Pentacles** | CV/gate generator driven by real 1D chaotic maps (logistic map and two relatives), not a disguised sequencer. |
| **StarLab** | Strymon StarLab-inspired: a Karplus-Strong string voice sharing a comb/allpass reverb tank with the effect side. |
| **Atlas** | The meta-synth: a library of designed sounds (wavetable, granular, modal, Plaits, supersaw, ladder...) all played with the same eight controls — CHARACTER, COLOR, MOTION, SPACE, SHAPE, ENERGY, TEXTURE and MORPH (see §3.3). |
| **Trio** | Three layered Plaits engines that split what you play: Full, Bass, Top or Arp per layer; pads play the scale or diatonic chords. |
| **Tinkertone** | An early-80s home keyboard with the MT-40's feature set (37 keys, 22 tones, 15-key bass, 6 rhythms) in an original look. Analog-modelled bass and drums, and you can record your own looping bass line (see §3.2). |
| **Orbit / Swarm / Mutant / Constellation / Dream** | Five Collection-engine instruments — see §7. |
| **Rings** | The real Mutable Instruments Rings resonator: modal bodies, sympathetic and inharmonic strings, FM voice, and the hidden "Disastrous Peace" string synth (see §3.5). |
| **Elements** | The real Mutable Instruments Elements: a bowed, blown and struck physical-modelling voice with its own reverb (see §3.5). |

### 3.1 Playing Plaits

Plaits opens on its **play view**; **R1** flips to the full parameter menu and
back. Notes keep sounding in both.

| Surface | What it does |
|---|---|
| Knob 1 / Knob 2 | Turn the current pair of dials (badged 1 and 2). Knob 1 press: next pair (Harmonics/Timbre → Morph/Decay → Attack/Release → Colour/Octave). Knob 2 press: reset the pair. |
| D-pad ▲▼ | Step through the 24 engines; the strip under the name shows where you are. |
| F2 | Pad layer: **Notes** (in key) → **Chords** (each pad is that degree's chord in the key) → **Controls** (16 parameters; tap one, knob 2 turns it) → **Moments** (tap to recall a whole sound, hold 0.6 s to store it). |
| L1 (hold) | Peek at the Controls layer without leaving the one you're on. |
| F3 | Arpeggiator on/off (it walks the held Notes pads). |
| Joystick | Bends Timbre (X) and Morph (Y) by up to half their range. **Click** keeps the bent sound: the offset is folded into the knobs. |
| Depth sensors (L2/R2 on a gamepad, the lenses on the sim frame, O/P keys) | Right hand: a theremin — plays on its own, pitch from height over 12 semitones, unquantised. Left hand: Harmonics. |
| MIDI keyboard | Real note numbers and velocity (louder and brighter), pitch bend (±2 semitones), mod wheel (Morph), aftertouch (Timbre). |
| Audio in | Optional envelope follower (Play Surface → Audio In) from **Hardware input** to any target. |

Every routing above lives in the menu's **Play Surface** group. Expression is
always an offset on top of the knobs, never overwriting them. Moments are
saved to `saves/plaits/moments.json` (the SD card on hardware). The camera is
not used by Plaits.

### 3.2 Tinkertone's bass line

The bass keys (F2 → **BASS**) play in three modes, set by **Bass** in the
menu or the **Bass Mode** control:

- **Manual** — the bass key you hold sounds.
- **Auto** — with the rhythm running, the held key is the root of a bass
  pattern in the **Auto Chord** you choose (Major / Minor / Minor 7th).
- **My Line** — your own recorded line loops with the rhythm.

To record one: turn **Record Line** on (it switches to My Line), set
**Line Length** (1, 2 or 4 bars), and play the bass keys. If the rhythm is
stopped, your first key starts it. Notes snap to the nearest step; holding
a key holds the note, and playing over an earlier take replaces only the
steps you play. Turn Record Line off to hear it loop. While it loops,
holding a bass key moves the whole line to start from that key, and a
Minor chord flattens its thirds and sevenths. **Clear Line** empties it.
The line is saved to the SD card (`saves/tinkertone/bassline.json`) and
comes back next time.

The power-on tempo is 84 BPM, in the 80–110 range where the original's
Rock rhythm found its second life in dancehall.

### 3.3 Atlas, the meta-synth

Every Atlas preset is a full synth patch (generators, filters, shapers and
effects wired into each other), but you always play it the same way:

| Surface | What it does |
|---|---|
| Knobs | CHARACTER / COLOR → MOTION / TEXTURE → SPACE / SHAPE → ENERGY / MORPH (knob 1 press: next pair). A macro the preset doesn't use shows "—". |
| D-pad ▲▼ | Next / previous preset. |
| MORPH | Glides every parameter through the preset's stored states, A → B (→ C → D). |
| Joystick | X pushes MORPH, Y pushes COLOR. Depth sensors: left = MOTION, right = SPACE. |
| Pads | Play the scale the preset sets (bottom-left is the root, blue pads are roots). |
| F2 | Pads → Controls → Moments (a Moment stores the whole sound: preset, states, macros, MORPH). |

R1 opens the menu. Its **Depth** row sets how much you see:

- **Play**: the controls above, plus level, voices, root, scale and transpose.
- **Edit**: adds every parameter by page and the envelope. It also adds
  **Store → A–D** (capture where MORPH is now as a state, adding one if
  needed), **Mutate**, **Revert** and **Save**, which writes a copy to
  `saves/atlas/presets/`.

  An edit changes the state MORPH is nearest to.
- **Deep**: adds the compiled graph (each block with its live activity), the
  patch's estimated cost, and the **CPU Budget**. Polyphony is capped to what
  fits in the budget, and **Voices** shows "fits N" when it bites.

The format and engine are documented in `docs/SYNTH_PLATFORM.md`. The same
engine runs Oracle.

### 3.4 Voltage presets

Voltage has **100 preset slots**. Slots 1–20 hold the factory presets,
twenty original synthwave patches; slots 21–100 start empty. A preset
holds the whole sound: oscillators, filter, envelopes, LFO, mono/glide,
the effects and the arp (so the arp presets start arpeggiating as soon as
you hold a chord).

| Where | What to do |
|---|---|
| Play view | The Controls layer's last pad is **Preset**: left/right steps through the slots. The status line shows the loaded preset when no note is held. |
| Menu → Presets → **Preset** | Left/right browses and loads. A `*` after the name means you've changed the sound since loading it. Hold SELECT to undo your edits. |
| Menu → Presets → **Save to** | Left/right picks the slot (it starts on the first empty one), SELECT saves. Saving over a preset keeps its name; an empty slot gets "User NN". Hold SELECT to clear the slot; a factory slot gets its factory preset back. |

Saved presets are written to the SD card at `saves/voltage/presets.json`
and come back after a restart. The factory set lives in
`assets/voltage/factory.json`; any field a preset leaves out takes the
init patch's value, so the file is easy to read and edit.

The factory presets:

| # | Name | # | Name |
|---|---|---|---|
| 1 | Neon Arp (arp) | 11 | Polaroid Sky |
| 2 | Night Drive Bass (mono) | 12 | Starfield Sweep |
| 3 | Outrun Lead (mono) | 13 | Coastline Lead (mono) |
| 4 | Sunset Pad | 14 | Tape Choir |
| 5 | Chrome Brass | 15 | Arcade Bass |
| 6 | Midnight Keys | 16 | Crystal Bells |
| 7 | Gated Pluck | 17 | Turbo Saw Stack |
| 8 | VHS Strings | 18 | After Hours Sub (mono) |
| 9 | Black Ice Bass (mono) | 19 | Rain Arp (arp) |
| 10 | Hyperdrive Arp (arp) | 20 | Final Lap Poly |

They are written for Voltage "in the style of" synthwave artists (each
preset's note in the file says which), not copies of anyone's patches.

**Mono** (Oscillators group) plays one voice at the newest held pad's
pitch; letting go of it returns to the pad still held. **Glide** slides
between notes played legato, up to 1 s. The **Effects** group runs
chorus → ping-pong delay → reverb, in stereo.

### 3.5 Mutable Instruments: Rings, Elements, Marbles, Tides

These four run the modules' own C++ code (from Mutable Instruments'
open-source eurorack repository, MIT-licensed), as Plaits and Clouds do.
Each opens on a play view with the module's panel on the dials; R1 shows
every control, plus the module's routing rows.

| App | Play it | Routing rows |
|---|---|---|
| **Rings** | Pads (chromatic from C3) or any keyboard strum it. Each note goes to the next of up to four voices. U/D picks the model. | **Exciter**: another app's audio excites the resonator instead of the internal exciter, like patching into the module's IN. |
| **Elements** | Pads play it; hold a note to keep bowing or blowing. A hand in the depth sensors bows (left) or blows (right). SPACE above 7/8 freezes the reverb. | **Input**: another app's audio strikes the resonator. |
| **Marbles** | F3 starts it (it opens stopped). X1, X2 and X3 play notes on T1, T2 and T3. | **Plays**: the instrument the notes go to. **T1–T3, X1–X3, Y App / Input**: patch any output to any app's mod input. |
| **Tides** | F3 starts it. Pads are its TRIG and V/OCT: they fire the AD/AR envelopes and transpose from C3, so in the Audio range they play it. | **Out 1–4 App / Input**: patch each output to a mod input. Looping slopes swing both ways around the knob; envelopes push one way. |

Why these four: they fill the gaps the other modules leave. Rings and
Elements are physical-modelling voices (Plaits has only a simple modal
engine); Marbles is a generative source for every instrument on the note
bus; Tides is the modulation source the mod bus didn't have. Grids, the
other obvious candidate, isn't here because its code is GPL-licensed and
Portamax is MIT. Braids is Plaits' predecessor, Warps is already here, and
Stages overlaps Tides and Pam's.

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
| **Mosaic** | Four effects in series (filters, delay, repeat, reverse, crush, drive, gate, ring, pitch, reverb), each switched and shaped by its own 16-step pattern. Pads = the focused slot's steps; hold one and turn knob 2 to lock its Amount. |
| **Squeeze** | Compressor with soft knee, makeup and parallel mix; pick another app as Sidechain to duck under it. |
| **Fracture / Ghosts / Tape Machine / Portal** | Collection-engine effects — see §7. |

## 5. Sequencing & modulation

| App | What it does |
|---|---|
| **Bloom** | Generative circular sequencer built from two groups of the same element ("dots"). |
| **Pam's** | Clone of the core of Pamela's Pro Workout — multi-channel clock/gate generator with logic combinators between channels. |
| **Turing Machine** | Music Thing Modular Turing Machine clone: clocked 16-bit shift register, "Locks" sets random-vs-repeat. |
| **Marbles** | The real Mutable Instruments Marbles: random rhythms and melodies with deja vu, played on any instrument (see §3.5). |
| **Tides** | The real Mutable Instruments Tides (2018): four linked envelopes, LFOs or oscillators (see §3.5). |
| **Sequencer** | Multi-track step sequencer in the spirit of Sugar Bytes DrumComputer. |
| **Ledger** | A tracker: 8 tracks × up to 64 rows × 16 patterns, each track a Plaits voice. Play view = performance (pads 1–8 mute, 9–16 loop / reverse / octave / half speed / dark); R1 = the editor (knob 1 rows, press for next field; knob 2 value, press to clear; pads enter notes). Effects: T M H D (sound locks), R retrigger, P chance, N nudge. |
| **CV Out** | 32 independent CV outputs sent as MIDI CC to an external MIDI-to-CV box. |
| **MIDI Learn** | Browse/add/remove CC → modulation-target mappings. |
| **Portal** | Eight-cable patch matrix: any registered source (LFOs, clocked random, audio buses) to any registered destination (Portal's own sends, or another app's modulation parameter). Direct monitor is off by default. Patches last for the session only — no patch-file persistence yet. |

## 6. Recording & library

| App | What it does |
|---|---|
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

## 8.1 Norns — run norns scripts

**Norns** runs scripts written for monome's norns, the Lua sound computer.
Copy a script folder to the SD card as `saves/norns/dust/code/<name>/`
(the same layout as norns' `dust/code`), open Norns, pick it from the
SELECT list and press SELECT. Twenty-one scripts are built in (see below).

| norns | Portamax |
|---|---|
| E2 | D-pad up / down |
| E3 | D-pad left / right |
| E1 | hold pad 16 and use the D-pad (or pads 9 / 10) |
| K2 / K3 | SELECT / hold SELECT, or pads 14 / 15 |
| K1 (hold) | pad 13 |
| PARAMS menu | pad 11 or F2 (up/down picks, left/right changes) |
| back to SELECT | pad 12 or F3 |
| MIDI notes in | pads 1–8 (C major from middle C, MIDI device 1) |

What works: the screen, encoders and keys, params (number, option,
control, taper, binary, trigger, groups), clock (`run`, `sync`, `sleep`,
tempo), metro, MIDI input, MIDI out (its notes go on the note bus, so a
script can play any instrument app), `musicutil`/`util`/`controlspec`,
the **PolyPerc** engine and **softcut** (6 voices, 2 × 60 s buffers,
loops, overdub, filters). Scripts keep running when you leave the app.

Not yet: other engines (a script that asks for one still runs, but its
engine commands are silent, and the status line says so), grid and arc
(scripts see them as unplugged), crow, audio input into softcut, and
saving params sets. awake runs unmodified.

**The built-in scripts.** All are original Portamax scripts written
against the documented norns API. Each one's controls are listed at the
top of its file (`assets/norns/code/<name>/<name>.lua`) and they make
good starting points for your own.

| Script | What it does |
|---|---|
| tidepool | Two tides; notes fall where they meet, into an echo |
| orbits | Four moons ring as they cross the top of their orbit |
| rainfall | Rain on tuned roof panels; wind pans it, K3 is thunder |
| cellular | A 1-D cellular automaton; each row plays its living cells |
| pendulums | A pendulum wave that drifts apart and realigns |
| rings | Three euclidean rhythms on three rings |
| markov | Learns which note follows which from the pads, then improvises |
| lsystem | A growing L-system grammar read as a melody |
| bounce | Balls in a box; where they hit sets the note |
| drone | A drifting chord drone layered into softcut loops |
| looper | Play the pads into a four-bar softcut loop and overdub |
| chordwalk | An arpeggiator over chord progressions; pads set the key |
| flock | A flock of birds; the highest ones sing |
| lorenz | The Lorenz attractor as pitch, pan and brightness |
| steps | A plain eight-step sequencer |
| chimes | Wind chimes in just intonation |
| warble | A melody into a wobbly tape echo |
| stacks | Falling blocks ring their column; a full row plays a chord |
| life | Conway's Game of Life, scanned column by column |
| spiral | Sunflower seeds placed by the golden angle |
| polymeter | A five-step bass against a seven-step melody |

## 9. Game controllers (PS5 and any other)

A DualSense, Xbox or MFi controller works on macOS through Apple's
`GameController.framework`; other USB/HID gamepads (and every controller on
Linux/Windows) go through `gilrs`. It needs the real app window focused to
be detected on macOS — an Apple platform requirement, not a bug.

What each button does is a mapping you can change in the **Controller** app
(Utilities): two maps, **Navigate** (home screen and menus) and **Play**
(apps on their play view). Select an action, press knob 2, then press the
button or move the stick/trigger you want; knob 2 left clears it; "Reset
this map" restores the defaults. Saved to `saves/controller_map.json`.

Defaults: Navigate is the original DualSense layout (D-pad left/right
browse, up/down edit, Cross select, Circle/Square/Triangle = F1/F2/F3, R1 =
Mixer, L1/Options/PS/touchpad = Home, right stick browses, left stick
unused). Play makes the pad an instrument controller: right stick = joystick
(R3 = keep), L2/R2 pressure = the two hand sensors, L1/R1 = the shoulders
(R1 toggles the menu), face buttons = the bottom row of pads, Options = F2,
Create = F3, D-pad = the device D-pad.

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
