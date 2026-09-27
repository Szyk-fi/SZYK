# Live collection apps

All 19 entries are installed in the main manifest-driven launcher. Run:

```sh
cd /Users/max/Developer/Modulade/portamax-sim
cargo run --example slint_home_live
```

These are Rust engines and live Slint panels. The older `slint_new_apps` gallery
remains a design study; it is not the executable instrument UI.

## Navigation and audio

D-pad up/down selects a parameter, left/right changes it, and R1 performs the
selected action. F1 goes home, F2 locks pads only for instruments that use them,
F3 supplies the current app's transport action, and F4 opens Mixer.

Effects and recorders start unpatched. Open an instrument to register its
output, then choose it in the receiving app's Source row. A source is tapped
before its mixer fader: lower its Mixer channel when you want only the effect's
output. Portal publishes a processed route; choose Portal as the destination
app's Source. Its Direct monitor defaults off to avoid doubling the signal.

For an external microphone/interface, select it in Settings → Input, then choose
Hardware input as the receiving app's Source. Input capture starts only after
that explicit selection; Off closes it. Stereo hardware channels are summed to
mono for the current AudioBus. Library audio playback and Studio pan remain stereo
at the main output. Hardware permission and device availability are handled by
the host OS. The simulator does not prove an embedded input driver.

## What each app does

| App | Implemented behavior |
|---|---|
| Orbit | Audio-clock polyrhythmic instrument sequencer; pads enable trigger points; ratio, pitch, gate, probability and swing affect notes. |
| Fracture | Real rolling-input slices with repeats, reverse, semitone pitch, scatter and dry/wet. Pads choose slices. |
| Ghosts | Four delayed generations of the input with geometric decay, drift, filtering and reverse reading. |
| Swarm | Up to 16 oscillators; cohesion, spread and mutation change detuning; pads play and F3 generates notes. |
| Mutant | Harmonic-parent blending, weighted partials and seeded patch mutation. R1 on Breed changes the actual synthesis coefficients. |
| Constellation | Chord degrees, inversions, octave spread, glide and tension determine real oscillator pitches. |
| Tape Machine | Four delay heads, variable head timing/speed, bounded feedback, wow/flutter and saturation. |
| Dream | Audio-clock Calm/Tension/Chaos/Release sequencing; state, dwell, transition chance, complexity and tension affect notes. Pads select state. |
| Portal | Explicit bus send with gain, polarity, filters and delay; optional direct monitor. |
| Reference | Stereo audio-file playback, seeking, gain, peak normalization, next-file playback and repeat. |
| Field | Selected-input recording with pre-roll, gain, limiter, low cut, optional monitor, marker and WAV export. |
| Sample Hunter | Input capture, threshold gate, trim bounds, normalized export and pitched audition. Exported WAVs enter the shared library. |
| Studio | Eight independent take buffers, track selection, levels, pan, mute/solo, overdub monitoring and stereo mix export. |
| Scope | Actual selected-input waveform/FFT, timebase, gain, rising-edge trigger, FFT window and display freeze; no audible output. |
| Vinyl | Audio collection browsing, file-name/newest sorting, variable-speed playback, repeat and automatic next file. No invented album metadata/artwork. |
| Practice | A–B looping with separate speed and pitch via overlapping grains, plus adjustable metronome. |
| Radio | User-configured HTTP(S) WAV streaming through a bounded queue, sample-rate conversion, gain/limiter, station repeat/next, and explicit connection/decoder errors. |
| Master | A/B audio-bus comparison, smoothed RMS matching, bass/presence processing, drive and output ceiling. |
| Memories | Shared recording library playback plus per-file favorite/tag sidecars. |

Recording pads: 1 Record/Stop, 2 Play/Stop, 3 Save WAV, 4 Marker. Studio pads 5–12
select tracks. F3 says RECORD for an empty take, STOP REC while recording, and
PLAY once audio is available. No input or no loaded file prevents false transport
activation. Save finishes a bounded snapshot before writing on a worker.

## Instruments and performance controls

Orbit, Swarm, Mutant, Constellation and Dream now have eight Instrument choices:
pure sine, glass FM, reed, plucked string, drawbar organ, round bass, live input
wavetable and imported audio sample. Each app retains its own sequencing/harmony
behavior. Tone color, attack, tail length, scale, octave and rate/depth of timbre motion
are editable below the original controls. Tail length scales the app’s own gate/release; motion changes the tone of every voice, including imported audio.

For your own instrument, put a mono/stereo audio file in `media/instruments/` (any media
subfolder works), choose **Rescan instruments / R1**, then **Sample instrument / R1**.
Left/right selects a file and R1 loads it. Samples are one-shots, pitched from a
C4 root; they are not multisample/SF2 instruments. For external audio, select
**Source / live input**; this selects the live-input voice. That voice treats the
current bus block as a wavetable; it does not track the input's musical pitch.

Every collection app has four **Recall scene / R1** recipes. Left/right chooses
a scene; R1 applies it. Routes, transport, loaded files and recordings remain
intact. Scenes recall parameters, so they can change levels/mutes. Synths also
recall voice, color, motion depth and release. Orbit/Dream can switch physical
pads between their original gates/states and keys; the other synths can switch
between keys and scene recall.

| App(s) | Additional performance features and live visual |
|---|---|
| Orbit | Four orbital trigger lanes; click a trigger or play gates/keys on pads. |
| Fracture | Sixteen playable buffer slices; Freeze input buffer preserves material. |
| Ghosts | Four memory layers; hold pads 1–4 to isolate generations; freeze memory. |
| Swarm | Voice population driven by actual oscillator phases and envelopes. |
| Mutant | Harmonic parents and offspring bars; click Breed to mutate real coefficients. |
| Constellation | Actual pitches mapped to a note circle; click a pitch class to change the root. |
| Tape Machine | Four head markers and recorded history; pads 1–4 isolate heads; freeze tape. |
| Dream | Click Calm/Tension/Chaos/Release to select a real engine state. |
| Portal | Eight independent cables: select a source, destination, bipolar amount, offset, slew and transform. Pads 1–8 select; 9–16 mute. |
| Reference | Actual loaded waveform, file title and playback progress. Pads 1–12 choose library entries; 13 Play, 14 Repeat, 15 Rewind, 16 Rescan. |
| Field | Real mono peak/RMS meters and take overview; pads 5 auto-gain, 6 monitor, 7 limiter, 8 low cut. |
| Sample Hunter | Drag the upper/lower waveform to set trim A/B. Click the key strip for pitched one-shot audition, or set Pads to Chromatic keys. |
| Studio | Eight real take lanes; click to select a track. Set Pads to Mute / solo: 1–8 mute, 9–16 solo. |
| Scope | Signed scope trace or measured spectrum. Pads 1 Freeze, 2 Wave/FFT, 3 Auto gain. |
| Vinyl | Click vector sleeves for real library files; same transport shortcuts as Reference. Empty libraries stay empty. |
| Practice | A/B loop window and playhead; pads 1/2 set loop points, 3/4 change speed. |
| Radio | Actual station name, buffer setting and audio spectrum; no decorative fake station frequency. |
| Master | Click A/B comparison; pad 3 Auto match, 4 Bypass processing. |
| Memories | Click an actual file in the journal timeline; favorites/tags still belong to each file. |

All graphics are Slint vector components fed by engine telemetry. They are not
screenshots or random demonstration signals. Visual updates are bounded to about
30 Hz; silent synth voices skip harmonic generation. Additional input voices,
media workers and app processors retain the existing lazy activation rules.

## Resource and fidelity limits

- Installing a manifest creates a dormant factory/proxy. No app engine, FFT,
  recording buffer, media scan or network request runs merely because it appears
  in the launcher. Removing one manifest does not require another app to exist.
- Open/playing/recording/routed apps remain active. Unused closed apps sleep after
  the existing grace period; new synth tails explicitly keep processing until
  quiet. Previously opened app state stays in memory to preserve takes/settings.
  This is lazy startup and DSP suspension, not a claim of automatic RAM eviction.
- Capture is bounded to 384,000 frames per track (8 s at 48 kHz). Studio can retain
  eight such stereo buffers; it is a short-take workstation, not disk streaming.
  Save snapshots copy at most 512 frames per callback; file encoding runs on a
  worker. Clear explicitly discards takes.
- Local playback supports WAV, FLAC, MP3, Ogg Vorbis, AAC/M4A/ALAC and AIFF, bounded to 5,760,000 frames (120 seconds at 48 kHz). Long-form local-file streaming is not implemented. Radio streams PCM/float WAV through a one-second
  queue; MP3/AAC stations are unsupported. Stop cancels the network connection;
  Repeat reconnects a completed stream.
- Practice uses lightweight granular stretch and can color transients; it is not
  a transparent commercial stretch engine. Delay/synthesis apps are original
  implementations, not certified emulations of external products.
- Studio exports the stereo mix; persistent multitrack sessions and per-track
  file export are not implemented. Memories tags/favorites use sidecars.
- Actual target RAM, CPU and audio-driver budgets still require measurement on
  the hardware. Desktop/software-render tests cannot establish those budgets.

## Verification of the playability update

On 2026-09-27, `cargo test --offline --bin portamax-sim` passed all 367 tests,
including 20 collection/streaming tests. New coverage checks voice differences,
motion on every voice, WAV-source silence without a loaded sample, scene bounds,
menu reachability, UI action routing, chromatic one-shot completion, two-source
crossfading, waveform thumbnails and distinct Fracture pads at zero Scatter.
The existing 100-installed-app lazy-startup/suspension test also passed.

The live example's `--render-instruments` path renders actual Slint panels and
exercises console controls without requesting an audio device. Its images are
software-render verification, not proof of physical audio quality or target CPU.

## System navigation and musical controls (September 2026)

The Slint launcher groups installed apps into Instruments, Effects, Sequencing,
Library and Utilities. All apps remains a complete list; Recent remembers the
last twelve opened apps during this session. Unknown/new apps appear under
Utilities automatically. The launcher reads metadata, not app constructors.
Up/down selects an app, left/right changes category, R1 opens. F1 opens Settings,
F2 cycles categories, F3 opens Recent and F4 opens Mixer. A green dot indicates
an app with an active transport. Session recents are not persisted to disk.

Scale-capable apps share sixteen scale families from the Berklee PULSE scale
reference: chromatic; major, natural minor, harmonic minor; Dorian, Phrygian,
Lydian, Mixolydian, Locrian; major/minor pentatonic and blues; whole tone; and
both diminished sequences. Focus Main scale or Root note to see its note names
and highlighted keyboard. Settings stay independent per app/voice/channel.
Bloom's custom scale remains available. Reference:
https://pulse.berklee.edu/scales/index.html

Orbit, Swarm, Mutant, Constellation and Dream offer Up, Down, Bounce, Random,
Random walk and Chords, combined with Straight, Offbeat, Euclidean 5/8, Sparse
or Bursts timing. Defaults vary between instruments. Constellation's chord mode
triggers its complete selected voicing; manual pads still play immediately.

Portal is now an eight-cable patch matrix. Installed audio sources are listed
immediately. Open a modulation destination app once to register its parameters,
then select a cable's Source and Destination. Sources include
two LFOs, clocked random and registered audio buses. Destinations include Portal's
main send, four auxiliary audio sends and registered modulation parameters.
Several cables can sum into one destination without replacing another modulation
writer. Amount is bipolar; Offset, Slew, Envelope, Gate and Sample & hold are
available per cable. Cable mute, Clear cable, Clear all and global Bypass remove
only Portal's contribution. Direct monitor is off by default; enable it to hear
the main send directly, or select a send as another app's audio input. Parameter
modulation is host-block-rate; audio sends preserve per-sample signals. This is
not universal audio-rate FM. Routing does not start another app's transport.
Patches currently last for the session; patch-file persistence is not implemented.

Retro now has four system tabs, a cartridge/library panel, a real game-frame
preview and explicit Load/Game view actions. Its emulator cores are unchanged
by this visual redesign. ROMs are still supplied by the user.

Compressed audio is decoded on a worker, retaining stereo and native sample
rate. Actual FLAC, AIFF and WAV fixtures are tested. Other enabled codec paths
rely on Symphonia; not every codec/container combination has a fixture. The
existing local-file frame limit applies to every format; oversized files show
an error instead of allocating unbounded memory. Radio streaming remains WAV.

Final verification for this update: all 377 binary tests passed, plus the
launcher's category/index/recent-order test. The live example's software-render
path completed for all 52 apps and all ten scale views, including Pam's enabled
quantizer. D-pad taps, 250 ms hold delay, repeat/release, joystick, shoulder
buttons, F1–F4 and all sixteen pads passed its input checks. Screenshots are
in the chat workspace's outputs/system-overhaul gallery.
