# Tinkertone

A no-frills recreation of the feature set of Casio's 1981 MT-40 home keyboard, in an original early-80s look.

| Section | What it does |
|---|---|
| Melody | 37 keys (C3–C6), 8 notes at once, 22 tones |
| Tone presets | 4 presets; each holds any of the 22 tones (Tone > Preset Holds) |
| Effects | Vibrato and Sustain switches |
| Bass | 15 bass keys (C2–D3), one bass voice; **Manual** plays what you hold, **Auto** plays a line that follows the rhythm from the bass key you hold, in Major, Minor or Minor 7th |
| Rhythm | 6 four-bar rhythms (Rock, Samba, Swing, Slow Rock, Waltz, Pops), tempo 40–240 BPM |
| Synchro Start | Arm it, and the rhythm starts with your first bass key |
| Fill-in | Hold it for sixteenth-note pulses of snare or kick; the two alternate press to press |
| Levels | Volume and Accompaniment volume (bass + drums) |

## Playing on Portamax

37 + 15 keys don't fit on 16 pads, so **F2** flips the pads between two layers:

- **KEYS**: 16 melody keys. Slide the window across all 37 keys with Pads > Key Window. Each C lights blue.
- **BASS**: pads 1–15 are the bass keys; pad 16 is Fill-in (hold).

**F3** starts and stops the rhythm. Other apps can modulate `Tinkertone: Volume`, `Accomp Volume` and `Tempo`, and process its audio as the `Tinkertone` source.

## How faithful it is

The feature set above is drawn from published descriptions of the original. These details aren't documented anywhere I could check, so they're best guesses, marked as such in the code (`src/apps/tinkertone.rs`): the full list of 22 tone names, exact key ranges, the vibrato depth and sustain length, and which 5 drum sounds the original had. The sound is a model of an early digital keyboard (8-bit additive wavetables, simple analog-style drums), not a capture of the real chip. The auto-bass lines are original; the famous factory "Rock" bassline is a composed work and isn't copied.
