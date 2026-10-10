# Rev2 capture plan

What to record on a real Prophet Rev2 so the open questions in
[PROPHET.md](PROPHET.md) can be settled from audio instead of guessed. Nothing
public covers these: no free raw Rev2 audio exists, and the two sources of
measurements (the Sequential forum thread by CreativeSpiral and the PresetPatch
technical appendix, same author) give cutoff, key tracking, LFO/matrix pitch
depths and amp attack times only.

Start every test from the Init program (Program > Initialize), effects off,
record dry, 24-bit, mono or stereo, one held note unless stated, 6 s of note then
release and 4 s of tail. Name files `<test>_<value>.wav` and note the MIDI note.

| # | Question | Test | Settings to vary |
|---|---|---|---|
| 1 | Filter shape and stopband | Saw, C2, both oscillators on | Cutoff 60, 80, 100, 120; 4-pole and 2-pole; resonance 0 |
| 2 | Resonance | Same | Cutoff 100, resonance 0, 32, 64, 96, 127 (4-pole) |
| 3 | Filter envelope amount | Filter env attack 0, decay 40, sustain 0, cutoff 40 | Amount 127 (neutral), 160, 190, 220, 254 and 100, 60, 0 |
| 4 | Filter audio mod | Osc 1 saw, osc 2 off, cutoff 60, resonance 0 | Audio mod 0, 16, 32, 64, 127 |
| 5 | Hard sync | Osc 1 C3, osc 2 saw, mix 127 (osc 2 only), sync on and off | Swap oscillator frequencies (osc 2 = 24, osc 1 = 36 and 48; then reversed) so the master is audible |
| 6 | Shape mod on saw, triangle, pulse | C2, cutoff 164 (open) | Shape mod 0, 25, 50, 75, 99 for each shape |
| 7 | Filter and amp envelope times | Cutoff 164, sustain 0 for decay; sustain 127 for release | Decay and release 20, 50, 80, 110, 127; filter attack 20, 60, 100 |
| 8 | Sub oscillator and noise | Osc 1 and 2 off | Sub 127; noise 127 (spectrum only) |
| 9 | Velocity response | Same note | Velocity 20, 64, 127 with filter and amp velocity at 0 and 127 |
| 10 | Oscillator mix law | Osc 1 saw C3, osc 2 saw G3 | Mix 0, 32, 64, 96, 127 |

Then, for end-to-end checks, ten factory or REVField programs, one held C3 each,
dry, with the program number written down. Add one plucked and one sequenced
program so the held-note replay can be replaced with a real one.

Analysis already exists: `reference_spectral_match` in
`src/apps/prophet/dsp.rs` compares third-octave spectra
(`REV2_DIR=<folder> REV2_DRY=1 cargo test --release --bin portamax-sim
reference_spectral -- --ignored --nocapture`). With known single notes the note
estimator is not needed and envelope timings can be fitted directly.
