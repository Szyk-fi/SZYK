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
  clickable keyboard: Slint uses press/release; the framebuffer simulator
  toggles a note when the same key is clicked again.
- L1 changes the edited layer. R1 moves through 16 sections. The first encoder
  selects a row; the second changes its value. Click a selected row to edit it.
- F2 opens Programs. F3 starts/stops the polyphonic sequences. Gated sequences
  run from held notes. Arpeggiation takes precedence over sequencing.
- Clicking the sequence-page strip advances the 16-step page and velocity
  track in the framebuffer panel. Slint has STEP and VEL TRACK buttons for
  these selections. All 64 steps and six note/velocity tracks are accessible
  per layer.
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
  jumping at control rate, and continues while voices are silent. Unmodulated
  A4 remains 440 Hz. Maximum slop now reaches ±4 semitones, with a quadratic
  amount taper and slow random targets. This approximates firsthand observations;
  the hardware's exact drift algorithm and amount curve remain uncalibrated.
- Triangle/random LFOs are bipolar; saw/reverse/square are unipolar. Free LFOs
  continue through silence. Key sync follows phrase starts. Independent layers
  use different oscillator phases and random seeds.
- Direct LFO pitch depth is 0.125 semitone per amount unit; matrix/auxiliary
  pitch depth is 0.5. Matrix routing to LFO amount uses the measured 4:1 scale.
  These depths follow published firsthand hardware measurements, not a claim
  of firmware equivalence. Gated pitch steps retain their half-semitone scale.
- Filter cutoff follows the measured semitone scale (105 ≈ 440 Hz without
  tracking, forum measurements by CreativeSpiral); cutoff 24 with keyboard amount 64 follows keyboard pitch. Audio modulation uses
  oscillator 1 and retains envelope velocity response. The filter no longer
  saturates its integrator memory every sample, restoring low-frequency gain.
- The sub is a square. Shaped ramps/triangles have slope-discontinuity
  corrections; shaped oscillators remove DC offsets. Hard sync and audio-rate
  filter modulation still need oversampling and hardware comparison.
- Auxiliary repeat cycles without an inserted release, and matrix modulation
  reaches its amount. VCA attack interpolates published measured timing anchors;
  other envelope stages remain generic software curves. Alternate pan modulation
  changes individual voice spread; Fixed moves the whole program. VCA modulation
  can open silent voices, and free polyphonic voices rotate by oldest use.
- Chorus supplies delayed wet taps without duplicating dry signal. Reverb uses
  eight coupled delay lines with separate stereo outputs and damped decay.
  These are original effects, not the Rev2's digital effect algorithms.

Native patch bytes and parameter IDs are unchanged. Correcting these behaviours
changes how existing patches sound. Before claiming hardware fidelity, compare
matched recordings for absolute cutoff, resonance/drive, oscillator shaping,
envelope/rate curves, slop depth and effects. Those are the main remaining sound
calibration gaps.

## Slint panel and launcher

The launcher sorts by visible name, so `prophet` appears alphabetically as Rev2.
The live Slint example uses `Rev2Panel`, not a generic parameter list or an image
of the framebuffer. It follows the earlier panel's wood sides, dark faceplate,
amber readouts, four knobs, patch toolbar and keyboard. All sixteen sections
and their parameter pages are connected to the real patch state, including layer
selection, Compare, Init and Save. Drag a knob vertically or click its +/− buttons;
pressing and releasing an on-screen piano key starts and stops its note. The
hardware framebuffer panel remains available in the non-Slint simulator.

Run `PORTAMAX_RENDER_APP=Rev2 cargo run --offline --example slint_home_live --
--render-instruments /tmp/rev2-ui` to render the actual UI, exercise pointer
callbacks and capture all sixteen sections. See [the detailed audit](REV2_AUDIT_2026-10-08.md)
for the parameter map, circuit research and exact remaining limits.

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
looping and amount modulation, free/synced LFO rate bands, absolute cutoff and
VCA attack anchors, pan destination modes, Multi channel routing, legal FX/unison/
gated/BPM limits, Slint paging/layer edits and pointer-note release, sequencing,
ties, gated slew, polyphony/priority, tunings and lock contention.

The optional external-reference test was run against Edisyn's
`SequentialProphetRev2.init` dump: both names and layers import, the exact
message exports unchanged, and the patch produces audio. The external file is
not committed. Run it with:

```sh
REV2_REFERENCE_SYX=/absolute/path/init.syx cargo test --offline \
  --bin portamax-sim reference_dump_imports_roundtrips_and_plays -- --ignored
```

Final 8 October audit: 1,201 broad-suite tests passed, 14 ignored, 3 excluded
(two sandbox-blocked UDP integration tests and the known stalled Norns bundled
script test). All 54 focused Rev2 regressions are included. The external init
test also passed separately; 440 imported user patches and all 16 original starts
render finite bounded audio. The Slint renderer verifies actual button/drag edits,
layer/page/tab controls, sequence step/velocity-track selection and key release.

## References

- [Sequential Rev2 guide 1.2.4](https://sequential.com/wp-content/uploads/2021/02/Prophet-Rev2-Users-Guide-1.2.4.pdf), especially Appendices A–E.
- [Edisyn Rev2 editor and raw format notes](https://github.com/eclab/edisyn/blob/master/edisyn/synth/sequentialprophetrev2/SequentialProphetRev2.java), Wim Verheyen, Apache-2.0. Used to cross-check wire facts, not as the sound engine.
- [Independent Rev2 parameter map](https://github.com/shimpe/sc-prophet-rev2/blob/master/Classes/ScProphetRev2.sc).
- [Firsthand Rev2 modulation-depth measurements](https://forum.sequential.com/index.php?topic=3203.0), CreativeSpiral. Used for pitch-route resolution, cutoff anchors, slop behavior and approximate VCA attack timing.

## Comparison with recordings (October 2026)

`reference_spectral_match` (ignored; `REV2_DIR=<folder> cargo test --release
--bin portamax-sim reference_spectral -- --ignored --nocapture`) reads
`REVfield1.wav`... (one clip per REVField program), estimates the notes in each
2 s window, plays them through the same program and compares third-octave
spectra after scaling both to equal total power, so loudness is ignored.
First ten programs: raising the cutoff scale by 12–15 semitones cut the mean
band error from 14.0 to 10.3 dB and removed the dark bias in eight of nine
usable clips. Remaining differences: programs 1, 5 and 10 stay dark at high
frequencies, program 9 (filter audio mod 30) is too bright, and programs 6 and
8 are plucks the held-note test cannot judge. Audio-rate filter modulation
depth (0.5 per unit) and the hard-sync direction are not settled; one clean
monophonic clip (program 10) preferred a depth of about 1.0.

### Filter leakage (October 2026)

Two sets of recordings were compared with the app: 110 dry REVField programs
(103 usable) and ten of Sequential's own factory demos matched to their programs
(Livid Saws, Knock Knock, Plush Pluck, Can't Kill Me, Blomp In the Night, Fast
Times At DSI, Marshmallow Pie, Repulsor Lift, Thx4TheMemory, Pizzicato; the
factory file is Sequential's `Rev2_Programs_v1.0.syx`, not distributed here).
The tool is the ignored `reference_spectral_match` test (`REV2_DIR=<folder of
REVfield<n>.wav> REV2_DRY=1 cargo test --release --bin portamax-sim
reference_spectral -- --ignored --nocapture`; `REV2_SYX` and `REV2_MAP` pair
clips with programs in any .syx, `REV2_STEP=n` takes every nth clip). It
estimates the notes in each 2 s window, replays them through the same program,
scales both spectra to equal power and compares third-octave bands, so loudness
is ignored.

With the measured cutoff scale (and, in an earlier attempt, an extra octave of
cutoff), sawtooth, pulse and saw+triangle programs were 10-25 dB too dark above
1 kHz and cutoffs under 60 were 30+ dB dark;
triangle programs fit. An ideal 24 dB/octave filter attenuates harmonically rich
waves far more than the hardware, so `FILTER_LEAK` adds 2% of the unfiltered mix,
low-passed at 1 kHz. A flat 0.4% leak fixed the dark bands but overshot above
4 kHz on the factory demos (+3 to +7 dB), hence the low-pass.

| Dry REVField set (103 clips) | mean band error | mean bias above 1 kHz | clips within 6 dB |
|---|---|---|---|
| octave shift only (earlier attempt) | 18.0 dB | -11.5 dB | 33 |
| flat 0.4% leak | 13.5 dB | -1.3 dB | 41 |
| 2% leak, 1 kHz low-pass | 13.1 dB | -1.9 dB | 43 |

With the leak in place a cutoff shift of 0 / +4 / +8 / +12 / +16 semitones gives
mean errors of 13.5 / 13.0 / 12.7 / 12.1 / 11.8 dB on the dry set and 12.6 / 12.3 /
12.4 / 12.4 / 12.6 dB on the factory demos, while cutoffs above 120 go from -2 dB
to +6 dB at +12; the shift was therefore removed and the measured scale kept.
On the ten factory demos the median band bias is within about 3 dB from 250 Hz
to 8 kHz and the mean error fell from 20.0 to 17.6 dB. Bias by oscillator 1
shape (dry set): saw -5.7, saw+triangle -4.5, triangle +0.7, pulse -2.3.

Remaining differences: cutoffs above 120 are about 8 dB too bright and cutoffs
under 60 about 5 dB dark; the 1-4 kHz bands stay 4-7 dB dark whatever the leak;
the filter-envelope sweep of low-cutoff programs may be under-scaled (positive
sweeps x1.5-2 with a larger cutoff shift improved that group but over-brightened
others); 0.4% flat versus 2% low-passed are fits, not the real circuit. Programs
with plucked or sequenced material cannot be judged by held-note replays.
