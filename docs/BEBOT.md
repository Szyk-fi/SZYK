# Bebot-inspired XY synth

An independent implementation of the playing concept in Normalware's Bebot,
not a port of its proprietary engine and not a claim of identical sound. All
robot artwork is original. The factory patches are newly designed here.

Open **Instruments → Bebot** in the live Slint simulator:

```sh
cargo run --example slint_home_live
```

Hold the left mouse button anywhere between the header and footer and drag.
X moves pitch through the selected range; Y controls lowpass cutoff for Saw
and Pulse, volume for Sine, and pulse width for PWM. Release stops the note;
release/cancel and leaving the app also clear the mouse gate. A mouse is one
finger; this desktop path does not implement four independent touchscreen
contacts. Seven additional pad/MIDI voices can sound alongside the mouse.

The header cycles eight factory patches (left arrow backwards, patch name
forwards), the oscillator and tuning mode. SETTINGS opens the sound editor.
Click the left half of a settings row to reduce it, the right half to increase
it. D-pad up/down selects a row, left/right edits. SELECT or joystick click
switches between settings and performance. SAVE USER and LOAD USER use one
user patch at `saves/bebot/user.json` (or `$PORTAMAX_SAVES_DIR/bebot/user.json`).
A user patch stores all 17 settings, including the musical scale and range.

Available: saw/pulse/sine/PWM; five scales and twelve roots; free, snap, slow
and fast pitch correction; 12–48 semitone ranges; resonance, pulse width,
PWM cutoff, attack/release, per-voice or post-mix overdrive, stereo chorus,
echo mix/time/repeats. Slow/fast correction is this implementation's gradual
approach to the nearest scale note; its curves are not calibrated to Bebot.

Pads ascend from bottom-left in the selected scale. MIDI/NoteBus notes keep
their original pitches, with +/-2 semitone pitch bend. Pad pressure/joystick Y
shape the pad voice; MIDI mod wheel shapes MIDI voices. Holding more than seven
pad/MIDI keys keeps the first seven active; extra held notes join when a slot
opens. The mixer and effect Source lists receive the declared Bebot output.

The actual editable Slint component is `examples/slint_common/bebot_panel.slint`.
It displays the same 640×360 UI as the device and forwards captured mouse
press/drag/release through the existing screen callback. The live shell imports
it and selects it when the active app is Bebot. No standalone mock UI is used.

DSP: PolyBLEP saw/pulse, sine, TPT state-variable lowpass, smoothed controls and
attack/release, stereo fractional-delay chorus and echo. Buffers are allocated
before playback; the audio thread uses a nonblocking control snapshot. Internal
engine maths are independent of the shared synthesis platform. Effects and
parameter ranges reproduce the musical idea, not undocumented engine internals.

Verification commands:

```sh
cargo test --bin portamax-sim apps::bebot
cargo run --example slint_home_live -- --render-bebot outputs/bebot
```

The second command dispatches real Slint mouse events, checks that a press
sounds, a drag changes frequency and release fades to silence, then saves
performance/settings previews. It uses a software renderer and no audio device.

Reference behaviour: https://www.normalware.com/bebotmanual/
