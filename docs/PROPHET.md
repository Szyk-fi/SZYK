# Portamax Rev2 software instrument

This replaces the unmerged Prophet-5-inspired prototype with a native
Prophet Rev2 patch architecture. Launcher: **Rev2**. App identifier: `prophet`; audio and mixer output: `Rev2`. It is working Rust synthesis, not a panel
mockup, Sequential firmware, or a validated circuit emulation.

## Load your patches

Place exported Rev2 `.syx` files in `patches/prophet-rev2/`, optionally in
subdirectories. Open Rev2, choose **Programs → Rescan .syx**, then turn the
Program row's value encoder. Single program dumps, edit-buffer dumps and
concatenated banks are supported. The browser keeps every imported program;
there is no 16-patch truncation. Invalid files are rejected atomically and
reported on the screen. The included 16 starting points are original sounds;
Sequential factory programs and paid sound sets are not distributed.

Both layers retain their names, parameters, gated tracks, polyphonic notes,
velocities, and unused bytes. The codec checks manufacturer `01`, Rev2 model
`2F`, commands `02`/`03`, 2046 raw bytes and 2339 packed MIDI bytes. NRPN
numbers are translated separately; they are not raw SysEx offsets.

**Programs → Save slot → Write SysEx** writes a hardware-format program dump
to `saves/prophet-rev2/U1-001.syx` through `U4-128.syx`. Rescan or restart to
recall saved programs. Native round trips preserve all transmitted bytes.
The hardware dump omits layer B's last two track-6 velocities; the editor
marks those unavailable instead of silently discarding an edit.

## Play and edit

- Pads and the device note bus play notes with velocity. The screen has a
  clickable keyboard; click the same key again to release it.
- L1 changes the edited layer. R1 moves through 16 sections. The first encoder
  selects a row; the second changes its value. Click a selected row to edit it.
- F2 opens Programs. F3 starts/stops the polyphonic sequences. Gated sequences
  run from held notes. Arpeggiation takes precedence over sequencing.
- Clicking the sequence-page strip advances the 16-step page and velocity
  track. All 64 steps and six note/velocity tracks are accessible per layer.
- Programs offers Compare, initialize, copy/swap layers and copy/swap poly
  sequences. Performance offers hold, transpose, 8/16 voices, mono, device
  clock following, step recording, master tuning and alternative tunings.
- To receive directly from a controller/hardware Rev2, use **Performance →
  Scan MIDI inputs → Direct MIDI input**. The default is Off; ordinary device
  note routing continues to work without opening a second MIDI port.
- Direct MIDI receives notes, velocity, bend, wheel, pressure, breath, foot,
  expression, sustain, program/bank selection, CC parameters, NRPN (including
  increment/decrement and null selection), clock/start/stop and Rev2 patch
  dumps. Multi mode routes the base channel to A and the next channel to B.
  For predictable multi-mode use, select a base MIDI channel rather than All.
- Saving exports a file. No program is automatically transmitted back to or
  written into your hardware synthesizer.

## Output level

Rev2 applies 12 dB of output makeup after the layer effects and before the
mixer gain/soft ceiling. This compensates for the conservative internal voice
and effect levels without changing filter drive, effect input levels, stored
patch volumes or their relative balances. Program Volume and Mixer → Rev2
Level still control loudness; dense chords are bounded by a soft
ceiling that stays linear below 0.3 and rounds overload peaks. Output gain is
a software calibration, not a hardware match.

## Sound audit, October 2026

Corrected playback faults affecting imported patches and original sounds:

- Pitch slop now moves slowly and independently per oscillator instead of
  jumping at control rate. Unmodulated A4 remains 440 Hz. The conservative
  slop range remains ±12 cents at maximum; hardware slop depth is uncalibrated.
- Triangle/random LFOs are bipolar; saw/reverse/square are unipolar. Free LFOs
  continue through silence. Key sync follows phrase starts. Independent layers
  use different oscillator phases and random seeds.
- Direct LFO pitch depth is 0.125 semitone per amount unit; matrix/auxiliary
  pitch depth is 0.5. Matrix routing to LFO amount uses the measured 4:1 scale.
  These depths follow published firsthand hardware measurements, not a claim
  of firmware equivalence. Gated pitch steps retain their half-semitone scale.
- Filter keyboard amount 64 tracks one semitone per key; audio modulation uses
  oscillator 1 and retains envelope velocity response. The filter no longer
  saturates its integrator memory every sample, restoring low-frequency gain.
- The sub is a square. Shaped ramps/triangles have slope-discontinuity
  corrections; shaped oscillators remove DC offsets. Hard sync and audio-rate
  filter modulation still need oversampling and hardware comparison.
- Auxiliary repeat cycles without an inserted release. Eight-voice fixed pan
  is balanced, and pan modulation moves voices consistently.
- Chorus supplies delayed wet taps without duplicating dry signal. Reverb uses
  eight coupled delay lines with separate stereo outputs and damped decay.
  These are original effects, not the Rev2's digital effect algorithms.

Native patch bytes and parameter IDs are unchanged. Correcting these behaviours
changes how existing patches sound. Before claiming hardware fidelity, compare
matched recordings for absolute cutoff, resonance/drive, oscillator shaping,
envelope/rate curves, slop depth and effects. Those are the main remaining sound
calibration gaps.

## Implemented playback architecture

| Area | Software behavior |
|---|---|
| Voices/layers | 8/16 voice allocation; A, stack and split; independent layer parameters/FX |
| Oscillators | Two DCO-style oscillators; off/saw/saw+triangle/triangle/pulse; shape modulation; fine/coarse tuning; sync; sub; noise; slop; key/reset switches |
| Filter | Resonant 2/4-pole low-pass, envelope, keyboard tracking, oscillator-1 audio modulation |
| Envelopes | Three delayed ADSRs, velocity amounts, bipolar filter/aux amounts, auxiliary looping/destination |
| LFOs | Four per voice; five waveforms; clock/key sync; independent rate, amount, destination |
| Modulation | Eight slots; all 23 source IDs and 52 destination IDs; fixed controller routes; amount-to-amount routing |
| Effects | Separate per-layer mono/stereo/BBD delay, chorus, three phasers, two flangers, reverb, ring mod, distortion, high-pass; mix/parameters/synced delay |
| Performance | Unison voice counts/detune; low/high/last priority and retrigger modes; glide modes; pan/spread; hold; transpose; master/program volume |
| Arpeggiator | Five orders, three octave ranges, repeats, relatch, clock divisions and swing |
| Gated sequencer | Four 16-step tracks, independent reset lengths, rest, five modes, track-2/4 slew |
| Poly sequencer | Two 64-step sequences; six notes/velocities per step; ties/rests; transposition; step recording/editing |
| Tuning | Equal temperament plus the guide's 16 alternative pitch tables; master coarse/fine |
| Buses | Existing notes, mixer, audio and eight modulation inputs per layer |

## Exactness and remaining gaps

**This is not yet an all-features-to-a-T Rev2 clone.** Native patch import is
lossless, but lossless import does not prove identical playback. In particular:

- Chord-memory storage is undocumented in the raw dump mappings consulted.
  Those bytes survive import/export, but imported chord voicings are not
  decoded. Chord mode currently uses ordinary unison and displays a warning.
  A hardware dump before/after storing a known six-note chord is needed to
  establish the layout rather than guessing reserved offsets.
- The oscillator shaping, filter, envelope/LFO curves, modulation depths and
  digital effects are original software models. They have not been calibrated
  against real Rev2 recordings. Sync/audio-rate modulation can alias.
- Direct MIDI controllers are shared between layers; independent per-channel
  controller state in Multi mode is not implemented. NRPN name edits,
  per-layer sequence start control, MIDI SysEx request/reply/librarian output,
  Prophet '08 patch conversion and custom MIDI tuning dumps remain unimplemented.
- Clock following uses the device tempo and transport; exact shared beat-phase
  alignment and all hardware slave/pedal modes remain to be completed.
- Step recording works; the hardware's complete real-time recording/overdub
  workflow, tap tempo, pedal polarity/response curves, local-control modes,
  calibration and physical USB/DIN/audio/sequence jacks are not emulated.
- This branch does not migrate the older prototype's JSON presets. That
  prototype was not merged into main. Its archived code/files remain available.

The current partial implementation was merged with Max's explicit approval.
These remaining gaps still prevent it from being described as a completed exact
clone.

## Validation

`cargo test --offline --bin portamax-sim apps::prophet` covers malformed and
concatenated dumps, realtime interleaving, independent packing vectors,
byte-for-byte export, save/recall, both-layer playback, pads/MIDI/release,
all original starting points at 8–192 kHz, oscillator/filter changes, four
LFOs/eight modulation slots, pitch/drift/depth/polarity, filter body/tracking,
DC/triangle alias reduction, stereo reverb decay, chorus wet-only behaviour,
independent layer seeds, free LFOs during silence, all effects, auxiliary
looping, sequencing,
ties, gated slew, polyphony/priority, tunings and lock contention.

The optional external-reference test was run against Edisyn's
`SequentialProphetRev2.init` dump: both names and layers import, the exact
message exports unchanged, and the patch produces audio. The external file is
not committed. Run it with:

```sh
REV2_REFERENCE_SYX=/absolute/path/init.syx cargo test --offline \
  --bin portamax-sim reference_dump_imports_roundtrips_and_plays -- --ignored
```

## References

- [Sequential Rev2 guide 1.2.4](https://sequential.com/wp-content/uploads/2021/02/Prophet-Rev2-Users-Guide-1.2.4.pdf), especially Appendices A–E.
- [Edisyn Rev2 editor and raw format notes](https://github.com/eclab/edisyn/blob/master/edisyn/synth/sequentialprophetrev2/SequentialProphetRev2.java), Wim Verheyen, Apache-2.0. Used to cross-check wire facts, not as the sound engine.
- [Independent Rev2 parameter map](https://github.com/shimpe/sc-prophet-rev2/blob/master/Classes/ScProphetRev2.sc).
- [Firsthand Rev2 modulation-depth measurements](https://forum.sequential.com/index.php?topic=3203.0), CreativeSpiral. Used for pitch-route resolution; other measured curves are not yet fitted.
