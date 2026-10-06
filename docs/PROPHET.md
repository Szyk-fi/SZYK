# Prophet

A complete, independent Portamax instrument with a black metal-style panel,
cream lettering, walnut side cheeks, red program display and live voice LEDs.
The same 640×360 picture and controls run in the framebuffer and Slint hosts.
The instrument is original Rust DSP inspired by the Prophet-5 signal path and
PikoPiko Factory's Profree-4 concept. It is **virtual analogue**, not Profree-4
firmware, a chip-level emulation, or an analogue circuit running on the MCU.

## Signal path

Each of up to eight voices has oscillator A (saw/pulse), oscillator B
(saw/pulse/triangle), independently selectable and combinable waveforms,
coarse tuning, B fine tuning, variable pulse width, A hard sync to B, noise,
a resonant four-pole low-pass, filter ADSR and amplifier ADSR. Oscillator B
can run at low frequency or ignore the keyboard for drones/cross-modulation.

Poly-Mod mixes the filter envelope and oscillator B, with independent routes
to A frequency, A pulse width and filter cutoff. This is per voice, including
audio-rate oscillator B modulation. Wheel Mod mixes the global LFO and noise
and routes to both oscillators' frequency/pulse width and the filter. LFO
shapes are triangle, saw, square, sine and sample-and-hold random.

Select four voices for the Profree-style allocation, five for the classic
Prophet layout, or eight for extra chords. Mono and unison use last-note
priority with return to a previously held note; poly mode steals released
voices first, then the oldest held voice. Unison has adjustable detune.
Glide, vintage pitch drift, keyboard filter tracking, velocity, stereo spread,
pitch bend range, drive, stereo chorus and ping-pong delay are included.

## Playing and editing

- Pads play chromatic notes from C3, bottom-left upward, with an octave control.
  MIDI keyboards and every sequencer's **Plays → Prophet** route work too.
- F2 cycles **panel → presets → menu → panel**. Pads remain musical on the
  panel/menu. On presets, pads 1–8 recall a sound and pads 9–16 select one of
  the eight factory banks. MIDI remains playable in the browser.
- The navigation encoder / D-pad up and down selects a control in the current
  section; the value encoder / D-pad left and right edits it. R1 advances the
  section; L1 advances the program. Encoder 1 press resets the selected control;
  encoder 2 press toggles Compare.
- Touch/click a section, switch or preset. Drag a dial up/down to edit it.
  The miniature keyboard plays notes while touched; releasing stops the note.
- Joystick X sweeps cutoff; joystick Y bends pitch. The left hand sensor adds
  wheel modulation. MIDI CC1 and channel aftertouch affect wheel amount/cutoff.
- **Compare** toggles between edits and the recalled sound. Editing while
  comparing first restores your edited sound. **Init** recalls a plain saw patch.
- Choose U01–U16 beside **Save**, then save an edited patch. Only that user
  slot is overwritten; factory patches are immutable. Files are versioned JSON
  in `saves/prophet/user-01.json` through `user-16.json`. Restarting reloads
  the saved bank. An invalid/version-incompatible file is not applied.

## Factory banks (64 original patches)

| Bank | Sounds |
|---|---|
| Brass | Walnut Brass, Soft Horns, Fanfare Five, Muted Trumpet, Toto Sunrise, Low Brass, Golden Stabs, Cinema Horns |
| Strings | Velvet Strings, Slow Orchestra, Silk Ensemble, Solstice Pad, Warm Tape, Night Choir, Fifth Dimension, Frozen Glass |
| Bass | Roundwood Bass, Rubber Pulse, Octave Bass, Low Voltage, Resonant Thumb, Unison Weight, Dark Triangle, Acid Timber |
| Leads | Ribbon Lead, Sync Skyline, Pulse Solo, Fifth Avenue, Portamento Gold, Reedy Mono, Wide Unison, Singing Saw |
| Keys | Wooden Tines, Copper Clav, Analog Harp, Short Circuit, Soft Mallet, Glass Keys, Midnight Piano, Rubber Marimba |
| Poly-Mod | Poly Bell, Crossmod Chime, Metal Bloom, Formant Wire, B Low Drone, Circuit Gong, Sync Brass, Broken Radio |
| Motion | PWM Clouds, Lighthouse, Random Tide, Pulsing Amber, Slow Sweep, Square Orbit, Noise Horizon, Afterglow |
| Essentials | Classic Saw, Twin Squares, Triangle Reed, Seventies Organ, Noise Snare, Analog Kick, Ocean Wind, Init Patch |

These are newly designed patches, not copied Sequential factory programs.
Each patch contains all 55 controls and can be exported by the optional demo
test below. A factory JSON can be edited, placed into a user-slot filename,
and loaded at the next startup. Parameter names must match and remain in range.

## Integration

Copy `src/apps/prophet/` and `apps/prophet/` into a current Portamax checkout,
then rebuild. `create()` and the manifest are auto-discovered by `build.rs`;
no central app list, synthesis engine, dependency or launcher edits are needed.
The manifest declares its audio output, note input and every modulation input.
**Prophet** appears in the launcher, mixer, effect Source pickers, modulation
pickers and sequencer Plays pickers, even before its screen is opened.

Audio uses fixed voice/state arrays and preallocated effects buffers. The
callback reads the keyboard snapshot and publishes audio with `try_lock`;
it keeps the previous snapshot if the UI is busy. Disk access and JSON parsing
are on the UI side. No callback allocations, blocking locks or file I/O.

## Validation and limits

`cargo test --bin portamax-sim apps::prophet` exercises sound/release, all
64 presets at 32/44.1/48/96 kHz, voice stealing, mono priority, sound-changing
sync/Poly-Mod/PWM/filter controls, patch validation, save/recall, Compare,
touch/MIDI/pad merging, mod/mixer/audio buses and UI lock contention.

```sh
PORTAMAX_PROPHET_EXPORT=/absolute/output/path cargo test --bin portamax-sim \
  export_previews_and_demo -- --ignored
```

This writes six real panel PNGs, all factory JSONs and a 32-second WAV demo.

Oscillators use PolyBLEP for the saw/pulse edges and the synth runs at 2× the
device sample rate. The filter is a feedback-solved TPT four-stage low-pass;
resonance is capped below self-oscillation. The triangle, hard-sync edges and
strong FM may alias at high pitches; the decimator is a two-pole low-pass,
not a brick-wall resampler. This is not an exact SSM2040/CEM3320 model, and
chorus/delay are extensions. Desktop validation does not prove an STM32N6 CPU
budget; profile the voice count and effects on the eventual hardware.

Behaviour references:

- [Sequential Prophet-5 user guide](https://sequential.com/wp-content/uploads/2021/02/Prophet-5-Users-Guide-1.3.pdf)
- [PikoPiko Factory Profree-4 project](https://www.kickstarter.com/projects/barbaraasuka/technical-release-project-for-profree-4-hardware-synthesizer)

No downloadable, licensed Profree-4 audio engine was found in the checked
public sources. If one becomes available, it can be assessed separately;
this app does not claim to contain that source.
