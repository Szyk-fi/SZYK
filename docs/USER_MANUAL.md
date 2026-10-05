# Portamax User Manual

Portamax is a self-contained groovebox/instrument simulator: 89 installed
apps sharing one audio engine, one clock, one modulation bus, and one 4x4 pad grid +
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
Library, Utilities, AI** and **Kids**. Up/down selects an app, left/right changes
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
| **Dexed** | A DX7-compatible 6-operator FM synth running Dexed's own engine — the DX7's envelopes, scaling, LFO and all 32 algorithms. Loads real `.syx` banks from `dx7_presets/` (none are bundled; it plays the DX7's INIT VOICE until you add some). Replaces the earlier Cascade (see §3.8). |
| **SoundFont** | Plays your `.sf2`/`.sf3` banks with TinySoundFont: any General MIDI bank or single instrument. U/D browses a bank's presets. Banks go in `soundfonts/` (see §3.8). |
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
| **Braids** | The real Mutable Instruments Braids macro oscillator: 47 models (saws, sync, filtered and vocal shapes, FM, plucked/bowed/blown, drums, wavetables, noises), TIMBRE and COLOR, an AD envelope on timbre/colour/pitch/VCA, bit and rate reduction (see §3.5). |
| **Chordsmith** | A whole chord under one finger, on any instrument: the bottom two pad rows are the key's chords (I ii iii IV V vi vii° bVII), the top two rows modify them (7th, sus4, add9, flip, 6th, power, bass on the fifth, wide). Each chord is voice-led from the last. Styles: pad, strum, arp up, arp up-down, pulse. **Plays** picks the instrument. |
| **Skins** | A drum synthesizer: eight synthesized voices (kick, snare, clap, hats, tom, rim, metal) with six controls each, 16-step lanes of independent length (polymeter), accents, ratchets, chance, and per-step sound locks. Four factory kits. Follows the device clock (see §5.1). Notes from 36 (C2) up play the voices. |
| **Chop** | A resampling sampler in the spirit of the SP-404 (see §3.6): record or load into 16 pads, chop at hits or into equal slices, two effect slots, and resample its own output. Notes from 36 play the pads. |
| **Looper** | Four loops locked to the device clock, with overdub, undo and redo (see §3.7). |

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
| **Braids** | Pads play it, one voice, last note wins; each note strikes it. U/D picks the model. | Turn **Env > VCA** off and it drones. |
| **Peaks** | Pads are its two gate inputs (lower two rows fire A, upper two B; notes from other apps too). Each of its two processors is one of twelve functions: envelope, LFO, tap LFO, bass / snare / FM drums, hi-hat, pulse shaper, pulse randomizer, bouncing ball, mini sequencer, number station. Four pots each. | The drums and the number station are audio; the rest are control voltages: **Out A / B App / Input** patch them to a mod input. |
| **Stages** | F3 starts it (it opens stopped). Pads are the gate input, high while held. U/D loads a preset group (ADSR, AR, LFOs, oscillators, pulse and gate generators, sample and hold, sequencer...). | The panel is the module's own: 1-6 segments, each with type, loop, slider and pot, so every preset stays editable. **Value / Phase App / Input**: patch its output to a mod input. The oscillator presets are audio. |
| **Tides** | F3 starts it. Pads are its TRIG and V/OCT: they fire the AD/AR envelopes and transpose from C3, so in the Audio range they play it. | **Out 1–4 App / Input**: patch each output to a mod input. Looping slopes swing both ways around the knob; envelopes push one way. |

Braids, Stages and Peaks follow the same pattern (below), and Streams is an effect (§4). Why these: they fill the gaps the other modules leave. Rings and
Elements are physical-modelling voices (Plaits has only a simple modal
engine); Marbles is a generative source for every instrument on the note
bus; Tides is the modulation source the mod bus didn't have. Grids, the
other obvious candidate, isn't here because its code is GPL-licensed and
Portamax is MIT.

### 3.6 Chop

F2 cycles four views. **SAMPLE**: pick the source (the hardware input,
or any app's output), how to chop (whole, 4, 8, 16 slices, or at each
hit, with a sensitivity), and press SELECT (or F3) to record; press again
to stop. The take lands on the picked pad, or across the pads from there
when chopped. **EDIT**: the picked pad's start, end, pitch, gain, mode
(one-shot, gate, loop) and reverse, and the **Library** row loads any WAV
from `media/`. **FX**: two effects in series on everything Chop plays —
vinyl, lo-fi, DJ filter, tempo delay, reverb, compressor, isolator, tape.
**PLAY**: the pads play; SELECT resamples Chop's own output, effects and
all, into the next empty pad. Banks save to `saves/chop/` as WAVs and a
JSON sheet.

### 3.7 Looper

Four loops, one per column of pads: row 1 records / overdubs, row 2
plays / mutes, row 3 undoes (and redoes) the last overdub, row 4 clears
(hold it). SELECT or F3 does record → close → overdub on the selected
loop; up/down picks the loop.

- **With the clock stopped**, the first recording starts the moment you
  press and stops when you press again. Closing it sets the tempo — the
  number of bars that puts it nearest 110 bpm — and starts the clock, so
  Session, Skins and anything else following the clock play along in
  time with what you just played.
- **With the clock running**, recording waits for the next bar (SETUP:
  *Start on* bar, beat or at once) and closes on a bar line, so loops are
  whole bars; *Length* can fix it at 1–16 bars instead. A press up to a
  quarter beat late closes the bar just passed.
- Loops are tied to the clock's beat, so they never drift and loops of
  different lengths phase exactly. Change the tempo and they follow like
  tape (the pitch moves too).
- SETUP: input, hearing the input, start/length, and per loop level,
  pan, reverse and how much an overdub keeps of what's there. Eight save
  slots, as WAVs in `saves/looper/`.

Not modelled: input latency compensation (the device knows its codec's
round trip; the simulator can't know the computer's).

### 3.8 Bring your own sounds

Several apps play content you supply; none is bundled, and none of it is
committed to the repository:

| Folder | For | Formats |
|---|---|---|
| `soundfonts/` | SoundFont | `.sf2`, `.sf3` |
| `dx7_presets/` | Dexed | DX7 SysEx `.syx` (a folder per bank collection) |
| `chiptunes/` | Chip Player | `.nsf` `.nsfe` `.spc` `.gbs` `.vgm` `.vgz` `.gym` `.hes` `.kss` `.ay` `.sap` |
| `orca/` | Orca | `.orca`; bundled examples are under `orca/examples/`, saved patterns under `orca/saved/` |

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
| **Streams** | The real Mutable Instruments Streams dual dynamics gate: per channel an envelope, vactrol, follower, compressor, filter controller or Lorenz generator turning an excite signal into gain. Pick each channel's **Audio** and **Excite** source in the menu (with no excite source the pads are the gate). | Gain and filter-frequency control voltages of both channels patch to any mod input. The module's analogue VCA is a digital one here. |
| **Squeeze** | Compressor with soft knee, makeup and parallel mix; pick another app as Sidechain to duck under it. |
| **Airwindows** | Chris Johnson's whole effect library — about 520 real plugins running their own code. Pick a **Source**, a Category (Airwindows' own grouping: Reverb, Tape, Consoles, Dynamics, Dithers...) and step through the Effect. Every plugin's parameters, units and value text come from the plugin itself. Pads and the stick/hands play the first parameters. |
| **Fracture / Ghosts / Tape Machine / Portal** | Collection-engine effects — see §7. |

## 5. Sequencing & modulation

| App | What it does |
|---|---|
| **Session** | The song layer: 8 tracks × 8 scenes of clips, each track playing any instrument app or Session's own sounds (drum tracks on the built-in kit). LAUNCH (pads launch clips and scenes on the next bar), STEP (edit steps: note, velocity, length, chance, lock), PLAY (play and record from pads or MIDI), SONG (chain scenes into an arrangement), SETUP (routing, mute, octave, lock-lane target, tempo, swing, clock, 8 project slots). Each track's lock lane sends a value per step to any app's modulation input. |
| **Tempo** | The device clock's front panel (see §5.1): tempo, tap tempo, play/stop, MIDI clock in and out, bar length, and a metronome. |
| **Bloom** | Generative circular sequencer built from two groups of the same element ("dots"). |
| **Pam's** | Clone of the core of Pamela's Pro Workout — multi-channel clock/gate generator with logic combinators between channels. |
| **Turing Machine** | Music Thing Modular Turing Machine clone: clocked 16-bit shift register, "Locks" sets random-vs-repeat. |
| **Orca** | The Orca livecoding sequencer: a grid of letters that run as operators and play notes. Edited by pad (D-pad moves the cursor, pads type glyphs, F2 turns the pages, SELECT erases); bundled examples, Save. Plays its own plain voice or any instrument on the note bus (see §5.2). |
| **Marbles** | The real Mutable Instruments Marbles: random rhythms and melodies with deja vu, played on any instrument (see §3.5). |
| **Tides** | The real Mutable Instruments Tides (2018): four linked envelopes, LFOs or oscillators (see §3.5). |
| **Sequencer** | Multi-track step sequencer in the spirit of Sugar Bytes DrumComputer. Per step (the step you last touched): pitch, rolls, delay, **velocity, accent, flam and chance**; per track: length, rate, probability, **accent amount**. The Drum instrument has the basic Kick/Snare/Hat/Clap plus **808 and 909 voices** (kicks, snares, claps, closed/open hats, toms, 808 rim and cowbell); a closed hat cuts an open one. The 808/909 voices model how those machines make their sounds; the 909's hats were ROM samples on the real machine, so here they are the same six-oscillator metal, brighter and tighter. |
| **Ledger** | A tracker: 8 tracks × up to 64 rows × 16 patterns, each track a Plaits voice or (its Plays row) any instrument app. Follows the device clock (§5.1). Play view = performance (pads 1–8 mute, 9–16 loop / reverse / octave / half speed / dark); R1 = the editor (knob 1 rows, press for next field; knob 2 value, press to clear; pads enter notes). Effects: T M H D (sound locks), R retrigger, P chance, N nudge. |
| **CV Out** | 32 independent CV outputs sent as MIDI CC to an external MIDI-to-CV box. |
| **MIDI Learn** | Browse/add/remove CC → modulation-target mappings. |
| **Portal** | Eight-cable patch matrix: any registered source (LFOs, clocked random, audio buses) to any registered destination (Portal's own sends, or another app's modulation parameter). Direct monitor is off by default. Patches last for the session only — no patch-file persistence yet. |

### 5.1 The device clock

There's one tempo and one transport for the whole device. Session,
Skins, Ledger and the Looper follow it (Session and Skins have a
**Clock** setting to run on their own tempo instead), so pressing play
in any of them starts all of them, locked to the same sample. Tempo is its front panel:

- **Tap**: any pad, in time (four taps or more average out).
- **Follow**: *internal*, or *MIDI clock in* — another device's clock
  (24 pulses a beat, Start, Continue, Stop and Song Position) drives the
  tempo and the transport. Jittery clocks are smoothed.
- **Send MIDI clock**: sends clock, Start, Stop and Continue to every MIDI
  output, so drum machines and DAWs follow the Portamax.
- **Bar**: beats per bar (the Looper's bar length and the metronome's
  accent).
- **Metronome**: off, on, or only while something records.

### 5.2 Orca and O&C

**Orca** runs Orca's own simulation. Every letter is an operator, every tick of
the device clock (a sixteenth note) runs the whole grid once. The pad pages are
the glyph keyboard: operators A–P, then Q–Z with `* # : ! ? %`, then the values
0–F, G–V and W–Z. `:` plays a note (channel, octave, note, velocity, length),
`%` a monophonic one. Notes go to the app named in **Plays** (menu: R1) or to
Orca's plain built-in voice. MIDI CC, pitch bend, OSC and UDP glyphs run but go
nowhere here.

**O&C** is the Ornaments & Crimes eurorack firmware running as the module's own
code: thirteen apps (quantizers, shift register, sequencers, envelopes, LFOs,
chords, Lorenz...) on its own 128x64 screen. The D-pad is its two encoders
(up/down the left one, left/right the right), SELECT the right encoder's button
(hold for the app list), the top pad row its four buttons, the second pad row
its four trigger inputs. CV 1–4 take a mod-bus input or the Controls knobs (the
stick and hands play them); triggers come from the pads or the mod bus. Its four
outputs patch to any mod input, and one output can play notes on another app.
Its settings persist in `saves/oc/`.

The **Firmware** menu row picks what each module runs, as if reflashing it: **Ornaments & Crimes** (the stock firmware), **Hemisphere Suite** (Chysn's fork: two applets side by side, each a small module — clocks, quantizers, envelopes, logic, sequencers, scopes) or **Phazerville Suite** (a large fork of Hemisphere with many more applets). Each module remembers its choice and keeps separate settings per firmware, so four modules can run four different firmwares. Hemisphere and Phazerville take the same controls; MIDI in/out inside them does nothing (no MIDI path into the firmware yet), and Phazerville has no DrumMap applet (it depends on GPL Grids data).

Four modules run at once: **O&C**, **O&C 2**, **O&C 3** and **O&C 4**, each its own firmware with its own screen, settings, inputs (`O&C 2: CV 1`...) and outputs. In the Slint shell, the menu's **Window** row opens a module in a window of its own, so all four can be on screen together; the window takes the keyboard: arrows turn the encoders (up/down the left, left/right the right), Return or R is the right button, U, D and L the other three, and 1–4 are the trigger inputs (held while the key is).

## 6. Recording & library

| App | What it does |
|---|---|
| **Sample Drum** | Erica Synths Sample Drum clone: dual-channel sample player/slicer (Start/Loop/End, four play modes, linear or zero-crossing slicing with FWD/BKW/RND/NONE/CV stepping, AHD envelope with Short/Mid/Long/Relative ranges and curve shapes, one insert FX per channel, presets). It opens with a sample on each channel, so pad 1/2 (TRIG) play straight away. **It also plays from notes**: a keyboard, sequencer or the note bus plays the sample at pitch (C4 = the sample's own pitch, velocity = level), to Ch 1, Ch 2 or both (Global ▸ Notes Play); looping modes fade out when the key is released. **SLICES pads** (F2 cycles TRIG / SLICES / SLICES 17+): one pad per slice of the selected channel, so a chopped break plays like a drum kit. **Tempo Match** (Sample ▸ Tempo Match, Loop Bars) speeds a loop up or down so its bars fit the project tempo, and Auto Clock then fires on that bar, so cutting a loop in 4, 8 or 16 all tile it; it turns on by itself for samples with a tempo in their name (`..._174_...`), and changes pitch with speed (no time-stretch). Tune is in semitones plus a Fine Tune in cents; retriggering fades the old hit out under the new one so it doesn't click. |
| **Chip Player** | Plays NES, SNES, Game Boy, Mega Drive/Genesis, Master System, PC Engine, MSX, ZX Spectrum and Atari chip music with real emulated chips (Game_Music_Emu). Files go in `chiptunes/`. F3 plays; the pads mute and unmute the chip's voices (see §3.8). |
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
SELECT list and press SELECT. 121 scripts are built in (see below).

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

**And 100 more**, all original, each with its controls at the top of its file:
acid, aeolian, aurora, beatty, bees, bellringers, billiards, breakout, breath, brownian, candle, canon, chance, chorale, chordmem, citylights, clapping, clockwork, collatz, comet, constellations, coral, crickets, dice, dragoncurve, drumcircle, dunes, fibonacci, fireflies, fractree, freezer, gamelan, geyser, glacier, grains, graycode, harmonizer, harp, heartbeat, hilbert, intervals, invaders, kaleidoscope, kalimba, koi, lander, langton, lavalamp, lighthouse, lissajous, logistic, loopdrift, lullaby, magnets, mandel, maze, metronomes, morse, multitap, musicbox, notedelay, pascal, pi, pinball, pingpong, pollen, pong, primes, radio, ratchet, reverso, ripples, riverstones, sandpile, scales, seasons, seismograph, sierpinski, slots, snake, snowfall, sonar, springs, starfield, strum, stutter, surf, tanpura, tapeloop, tempo, thunderhead, traffic, train, tron, tuplets, typewriter, volcano, voronoi, walkingbass, zengarden.
They cover nature and physics (snowfall, geyser, metronomes that fall into step), maths (Langton's ant, the dragon curve, primes), music (canon, chorale, bell-ringing changes, gamelan, a walking bass), tape and softcut tools (tape loops, stutter, multitap) and games that play themselves (pong, invaders, lander).

### 8.2 Ableton Push 2 lights

With a Push 2 plugged in, the bottom-left 4×4 of its pads are Portamax's
pads, and the four pads above them are F1–F4. Their lights always match
the pads on screen; both come from one rule (`src/pad_lights.rs`):
- a pad you're holding is red;
- otherwise it shows the app's own colour (the Sequencer's steps, the
  play view's layers, C notes in blue on the keyboards);
- F1–F4 are always lit: yellow, green, blue, red.

Plug the Push in or power-cycle it at any time: Portamax notices the MIDI
port change within a few seconds and sends every light again. It also
refreshes all of them every few seconds, so a dropped message never
leaves a pad wrong for long.

### 8.3 Kids

Ten apps in the launcher's **Kids** section, five for ages 6-9 and five
for ages 10-12. Each draws its whole screen itself, with big pictures and
a line of help along the bottom; F1 always goes home. They share one small
sound engine (`src/apps/kids_kit.rs`): a glockenspiel and marimba built
from their bars' real overtones, a Karplus-Strong harp, organ, flute,
chiptune and bass voices, a synthesized drum kit (the hi-hats and cowbell
use the TR-808's circuits), a room reverb and a sample-accurate clock.
Each app has its own Mixer channel and audio output, so Studio can record
it.

**Ages 6-9** (little reading needed):

| App | What it does | Pads | Other controls |
|---|---|---|---|
| Rainbow Bells | A rainbow glockenspiel in the major pentatonic, so every note fits | 16 bars, lowest bottom-left | left/right sound (bells, marimba, harp, flute); up/down octave; SELECT records a loop, again to play it, again to add more; hold SELECT erases; F3 plays/stops the loop |
| Critter Choir | Animals that sing: a farm choir (cow, dog, duck, cat) and a pond choir (frog, owl, bee, bird) | each column is a critter, bottom pad lowest | left/right other choir; F3 the choir sings a song you can join |
| Copy Cat | A memory game: the cat plays a tune on four coloured mats, you copy it | the four corners are the mats | SELECT starts; three misses end a game |
| Bug Beats | A first drum machine: put a bug on a beat and it plays | rows are sounds, columns are the four beats | left/right band (Drums, Jungle, Party); up/down speed; SELECT play/stop; hold SELECT clears |
| Monster Mic | A voice changer: Me, Robot, Chipmunk, Monster, Alien, Cave, Ghost, Underwater | top two rows pick a voice; pad 9 held records five seconds; pads 10-16 play it back at seven pitches | up/down live mic level (bottom = off); left/right input |

Monster Mic listens to the hardware input once one is chosen in
Settings → Input. With the internal speaker the mic can feed back:
headphones, or the live mic turned down, stop that.

**Ages 10-12** (each one explains what it's doing):

| App | What it teaches | Pads | Other controls |
|---|---|---|---|
| Beat Lab | Rhythm and genre: 4 tracks × 16 steps, swing, seven styles (rock, boom bap, house, reggaeton, drum & bass, funk, bossa nova), each with a line on what defines it | the 16 steps of the selected track; tap cycles on, loud, soft, off | up/down menu (tracks, tempo, swing, style); left/right changes; SELECT play; hold SELECT clears the track |
| Chord Garden | Harmony: a four-chord progression in Roman numerals, coloured by function (home, away, pull home) | row 1 pick a chord slot; rows 2-3 the key's seven chords and a 7th; row 4 melody notes that always fit | up/down Key / Style / Tempo; styles Pads, Guitar, Arpeggio, Piano; hold SELECT loads a well-known progression |
| Sound Detective | Synthesis: match a mystery sound's wave, octave, filter brightness, attack and length; a level hides one more control each time | 1 hear the mystery, 2 hear yours, 3 check, 4 new case; the rest play your sound | up/down pick a control, left/right turn it; SELECT checks |
| Ear Quest | Ear training in five quests that unlock in turn: higher or lower, major or minor, same or different, intervals, which solfa note | the answers (halves for two, corners for four) | SELECT hear again; up/down quest; hold SELECT skip |
| Music Code | Programming: a tune as blocks (PLAY, REST, CHORD, DRUM, UP, DOWN, LEAP, DICE, HOME, REPEAT, IF HIGH) run one step at a time | top three rows insert blocks at the cursor; bottom row cursor left/right, delete, run | left/right cursor; up/down instrument; hold SELECT clears |

### 8.4 AI apps (the NPU)

Seven apps in the launcher's **AI** section, each built around a small
neural network made for the STM32N6's Neural-ART NPU. The networks are
trained in `tools/npu` (see its README) on data synthesized there,
quantised to int8, and exported twice: a `.pmxn` file the sim runs, and
an ONNX file for ST Edge AI to compile for the chip. The sim runs them on
the computer's CPU with the same int8 maths, and its tests check the
results bit for bit against the Python reference. Each app's bottom line
shows how many inferences it runs, how much work that is, and roughly
what share of the NPU it would use.

| App | What the network does | How you play it |
|---|---|---|
| **Hum** | Pitch tracking: 100 times a second, which of 145 pitches (a third of a semitone apart, C2-C6) it hears, or none. Within 50 cents 99% of the time on its test set; the screen is also a tuner. | Sing, hum or play into the input. **Plays** sends the notes to Hum's own sound or any instrument. Scale and key snap the notes; Glide mode follows your voice continuously instead. |
| **Mouth Drums** | Sound embedding: the first 80 ms of each sound becomes 64 numbers; your taught sounds are averaged into prototypes and each new sound goes to the closest. 98% right picking among 8 sounds taught 3 times each (test set). | Hold one of pads 1-8 and make a sound 3-5 times to teach it; then beatbox. Pad 9 records into a 2-bar loop, 10 plays, 11 clears, 12 click. You hear each hit about 80 ms after you make it (the network needs that much of the sound). |
| **Conductor** | Gesture recognition on the two depth sensors: swipe right/left, push, wave left/right, tap left/right, 20 times a second. Holding still to play is not a gesture. | Six outputs (left hand, right hand, swipes, push, waves, taps) patch to any modulation input. Its own pad shows the gestures while your hands are over the sensors. |
| **Timbre Map** | A neural synthesizer: a decoder turns a point on a 2-D map of 16 instrument families, the note, the velocity and the time into 32 harmonic levels, 4 noise bands and a loudness, every 4 ms per voice, played by an additive synth. | Pads, MIDI or other apps play it. D-pad, joystick or hands move across the map (moving morphs sounding notes); SELECT jumps to the next landmark, hold SELECT drifts. |
| **Band Mate** | Chord recognition: 10 times a second, which of 24 major and minor chords is playing, or none (94% on its test set, including sevenths and inversions). | Play chords into the input; drums and bass follow (Rock, Ballad, Funk, Reggae, Shuffle). The band comes in on your first chord. Pad 1 taps the tempo. |
| **Choir** | Hum's pitch tracker, for a vocal tuner and harmoniser. The shifting is TD-PSOLA, which keeps the voice's formants, so harmonies sound like a second singer rather than sped-up tape. Every voice comes out 33 ms late, together. | Sing into the input. The lead is pulled into the key (Tuning amount; Retune speed, down to instant for the hard-tuned sound). Pads pick two harmony intervals (top rows voice 1, bottom rows voice 2), always in the key; Harmony "keys" sings the notes you hold on a keyboard or that a sequencer sends instead. |
| **Sorter** | Sound classification and similarity: a sample's first 300 ms becomes 48 numbers ("sounds like") and a name — kick, snare, clap, closed or open hat, cymbal, tom, rim, metal, shaker, hand perc, bass, tonal, texture. 97% on its synthetic test set; on drums from synths it never heard, 13 of 14 exactly right. | It hears every WAV in `media/` once (remembered in `saves/sorter/`). MAP: the library laid out by similarity; the D-pad walks, the pads play the 16 most alike. LIST: kind by kind. KIT: SELECT builds a 16-pad kit around the sound you're on, left/right for other takes, down exports it to `media/Kits/`. |

The listening apps (Hum, Mouth Drums, Band Mate, Choir) start on the device's
hardware input, which only listens once an input is chosen in Settings →
Input. The network always runs on a worker thread, never the audio
thread, as it would on the NPU.

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
