# Dynamic visualizers — first implementation

Run `cargo run --example slint_home_live` from the project directory.

## Vector Filter

Open **Effects → Vector Filter** and choose any installed audio output in Source. Hardware input is available after selecting
an input device in Settings. Starting the source's transport remains explicit.

Drag the front face: X maps logarithmically to 40–16,000 Hz cutoff; Y maps to
resonance Q 0.5–8. Move the Z strip to set 1–8× drive. The cube's control point
shows the three current values; its signal trace is actual filtered output.
Left/right on the corresponding menu rows works with the console or MIDI controls.
Low-pass, band-pass and high-pass modes share the same controller. Wet sets the
processed balance; F3 bypasses/enables processing with a short crossfade.

The filter is an independent lazy app. It publishes an AudioBus output and
three normalized additive modulation destinations (Cutoff, Resonance, Drive),
which Portal can address after the app has been opened. Unpatched startup is
silent. Mixer controls its final level. Removing its manifest does not affect
other apps. It is a projected 3D vector interface; it does not require a GPU
mesh renderer or a continuously running visual background task.

## Scope — Spectral Terrain

Choose a Source, then set Display mode to **Spectral terrain** (or press pad 2
to switch from Waveform). Frequency runs left to right; depth shows the last
12 displayed spectral measurements. The mesh contains 32 bins per row and
460 bounded line segments. Freeze holds both the measurement and its history.
Waveform mode remains available. The display uses the existing FFT; it adds
no second FFT or synthetic oscillator.

## Swarm — Voice Network

Swarm's old dot field is replaced by a bounded network of up to sixteen voices.
Pitch changes radial position, oscillator phase moves nodes, and envelope level
controls node/connection brightness. Cohesion changes the cluster spread.
Touch a node to play its voice; pads and the existing generator still work.
The connections depict musical voice relationships, not additional DSP routes.

## Specialized app views (now implemented)

- **Interference Loom** for Ghosts: stacked signed loop traces, woven by each
  memory layer's age and decay; touch a strand to isolate that layer.
- **Prismatic Shards** for Fracture: a faceted view of the captured samples in each
  playable slice; reverse flips its direction and touch punches a slice.
- **Harmonic Orrery** for Constellation: nested chord planes, pitch-class links
  and envelope illumination; drag the field to change inversion or octave spread.

The gallery captures real Slint components. Its Scope input is an explicitly
named multitone verification signal. The filter capture uses that registered
verification source. Physical audio quality and embedded CPU/GPU budgets still
require target-device testing.

## Implemented visualizer suite

Scope now has ten Display modes: its original Waveform and Spectral terrain,
plus **Interference Loom, Prismatic Shards, Harmonic Orrery, Phase Portrait,
Spectral Crown, Spectrogram, Contour Field, and Wave Ribbons**. Pad 2 cycles all
ten. Gain and timebase shape waveform views; Hann/rectangular window shapes the
spectral views. Freeze holds all measurements and history. Phase Portrait is a
mono signal versus delayed copies, not a stereo correlation measurement. Scope's
Orrery shows spectral bands; Constellation's version shows actual note voices.

Ghosts' Interference Loom reads four signed delayed memory windows with the
engine's age, drift, reverse and decay applied. Hold a SOLO target or pad 1–4 to
isolate generations. Fracture's Prismatic Shards show sixteen actual slice
windows; hold a facet to punch its slice, with Reverse flipping the facet's
orientation. Constellation's Harmonic Orrery positions voices by note class and
octave, with envelope-driven illumination. Drag across the orbit field to edit
inversion horizontally and octave spread vertically; the labelled strips also
edit root, inversion and spread through the same engine parameters.

The new vector views use at most 560 line segments, constructed from bounded
telemetry on the UI thread. They introduce no extra FFT, renderer thread or
background animation timer. Silence produces quiet/static geometry, rather
than invented signal activity. Target-device frame rate remains unverified.

## Installed-source catalog

Every installed audio-producing app declares its output ports before its engine
is constructed. Bloom and the other instruments therefore appear in compatible
Source lists immediately. CV-only services are not falsely listed as audio.
Reading a patched buffer requests its owner; a UI background tick constructs
that app lazily and enables its DSP. Source lists and Mixer meters never request
engines. Disconnected demand expires after 500 ms, while existing active-screen,
transport, recording and note-tail rules still apply. No transport or microphone
is automatically started by selecting a source. Hardware input stays controlled
by Settings. Removing an app's manifest removes its catalog entry on restart.

Analyzer and Visualizer now have Source rows and measure actual bus audio.
Monitor defaults off in the main system to avoid doubling audible input; their
bus output remains a pre-monitor tap. Scope likewise passes its input to its bus
output while remaining inaudible itself. The three input mixers allocate enough
slots for the installed catalog (with spare capacity), rather than truncating
installed sources at 128. Adding a new engine requires declaring its output
names in the registry's installation contract. Runtime hot installation is not
implemented.
