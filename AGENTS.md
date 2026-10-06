# AGENTS.md: shared rules for AI assistants on Portamax

Two AI assistants work on this repo, Claude and ChatGPT/Codex. Neither can see
the other's conversations. **This file and git history are the only shared
channel.** Read this whole file before starting any work. Add an entry to the
Work log before you finish.

Max is the owner and the go-between. When in doubt, stop and ask Max. Don't
guess what the other assistant meant.

General project context (what Portamax is, the `App` trait, the "no fakes"
rule, how to add an app) is in `CLAUDE.md`. Read it too: it applies to both
assistants, not just Claude.

---

## 1. Before you start

1. `git status`. Don't start on a dirty tree you didn't make. Ask Max what it
   is. (`samples/` is always untracked; ignore it.)
2. Read the **Work log** (section 5), at least the last few entries, and
   check **In progress / locks** (section 4).
3. Check **Ownership** (section 2). If the task touches the other assistant's
   area, stop and tell Max before editing.
4. Work on your own branch, cut from the branch Max names:
   `claude/<topic>` or `gpt/<topic>`. Never commit straight to `main`.

## 2. Ownership

Each area has one owner. The owner may change it freely. The other assistant
may read it but must not edit it without Max's go-ahead.

| Area | Paths | Owner |
|---|---|---|
| Synthesis engine | `src/synthesis/` (patch, blocks, expr, engine, evolve) | Claude |
| Atlas app + presets | `src/apps/atlas.rs`, `assets/atlas/` | Claude |
| Oracle app | `src/apps/oracle/` | Claude |
| I/O cards | `src/io_cards.rs`, `src/apps/cards.rs`, `assets/io_cards/`, `saves/io_slots.json` | ChatGPT |
| Hardware / KiCad | schematic, PCB, pin plan (not in this repo) | ChatGPT |
| Other apps | everything else in `src/apps/` | whoever Max assigns; log it in section 4 |
| Shared / cross-cutting | `Cargo.toml`, `src/audio_bus.rs`, `src/note_bus.rs`, `src/modbus.rs`, `src/mixer_bus.rs`, `src/app.rs`, `src/registry.rs`, `src/apps/mod.rs`, `apps/*/manifest.toml`, `docs/`, `CLAUDE.md`, this file | **Max decides per change** |

*(Proposed split. Max: edit the Owner column however you like.)*

**Cross-boundary changes.** If your work needs a change in the other's area,
such as a new field on a shared type or a new bus source kind:
- don't make it;
- add a **Request** entry to the Work log that says what you need and why;
- tell Max.

## 3. Contracts both assistants must respect

These are load-bearing. Breaking one breaks the other assistant's work. Full
details are in `docs/SYNTH_PLATFORM.md` and `docs/IO_CARDS.md`.

**Synthesis**
- The audio thread never blocks, allocates, frees or panics. All validation
  and allocation happens in `compile`, off the audio thread. Engines are
  swapped through the `try_lock` slot with a crossfade, and the old engine is
  freed on the UI thread.
- `blocks::SPECS` is the single source of truth for blocks. Oracle's AI prompt
  is generated from it. Changing a spec changes what Oracle writes.
- The patch format is versioned. Any format change bumps `CURRENT_VERSION` and
  adds a `migrate` step. Never break loading of older patches.
- A new block needs four things: a spec, a `tick` arm, a `cost`, and a
  behaviour test.
- The Atlas macro names are fixed: CHARACTER COLOR MOTION SPACE SHAPE ENERGY
  TEXTURE, plus MORPH.

**I/O cards**
- Only digital signals cross the M.2 connector.
- EEPROM layout v1 is frozen. A change means a new format version, not an edit
  to v1.
- `SLOT_LANES` decides which slots accept which cards. Audio is on A and B only
  in rev A.
- +5 V is enabled only after a valid EEPROM read and a budget check
  (`FIVE_V_BUDGET_MA`).
- Cards plug into the **existing** audio, note and mod buses. Apps should need
  no changes.

## 4. In progress / locks

List anything half-finished, or a file you need nobody else to touch for now.
Remove your line when you're done.

| Who | What | Files | Since |
|---|---|---|---|
| | | | |

## 5. Work log

Newest at the top. Keep each entry short. Use this format:

```
### YYYY-MM-DD: <Claude|ChatGPT>: <one-line summary>
- Branch: <branch>   Commits: <short hashes>
- Changed: <files / modules>
- Status: done | partial (what's left) | blocked (on what)
- Tests: <what you ran, pass/fail>
- Notes for the other assistant: <anything that affects them, or "none">
- Request: <only if you need a change in the other's area>
```

### 2026-10-06: Claude: grid over USB serial (mext), no serialosc needed
- Branch: claude/teletype
- Changed: src/apps/grid_kit.rs (new `mext` codec module, new `serial` driver,
  serialosc now marks itself heard and won't attach over a USB-attached grid),
  src/apps/grid.rs (one status string), Cargo.toml (shared: `libc = "0.2"`
  under cfg(unix), already in Cargo.lock; raw termios for the serial port).
- Status: done. A grid plugged into the Mac is found on /dev/cu.usbserial-m*
  or /dev/cu.usbmodem* (ttyUSB*/ttyACM* on Linux), probed with mext's size
  query, opened exclusively at 115200 raw; keys in, changed 8x8 level maps
  out. If serialosc answers it owns the grid and the driver stays off the
  ports. `mext` is allocation-free plain bytes, meant for the firmware's USB
  host too. Protocol details from libmonome (ISC).
- Tests: cargo test --bin portamax-sim (adds a fake grid on a pty).
- Notes for the other assistant: hardware: a grid on Portamax needs USB host
  mode on OTG1 with VBUS out (older grids are FTDI, newer CDC-ACM).

### 2026-10-06: Claude: the grid (on-screen + real monome via serialosc), Kria on it
- Branch: claude/teletype
- Changed: new src/apps/grid_kit.rs (shared grid hub, OSC codec, serialosc
  client thread), new src/apps/grid.rs + apps/grid/manifest.toml (Grid app),
  src/apps/kria.rs (Ansible grid layout as a grid client), and
  examples/slint_home_live.rs: a TouchArea on the full-screen picture
  (active-kind 43) that sends clicks to the active app's slint_pointer_pick in
  640x360 picture pixels, (-1,-1) on release. No other app implements
  slint_pointer_pick for that kind, so nothing else changes.
- Status: done. Grid 8x16 by default, 1x1..64x128; Follow hardware sizes it to a
  plugged-in grid; the hardware shows a movable window of a bigger grid.
  Not done: arc, Teletype G.* ops, norns `grid` API (prelude.lua still
  reports no grid).
- Tests: cargo test --bin portamax-sim (incl. a loopback serialosc exchange).
- Notes for the other assistant: apps that want the grid call
  grid_kit::grid().register(name), read keys(name) and show(name, &Leds) on the
  UI thread (background_tick). Never from the audio thread.

### 2026-10-06: Claude: Teletype and Kria (monome) apps, KR.* ops
- Branch: claude/teletype   Commits: b048ebc (Teletype), d3fe370 (Kria + KR ops)
- Changed: new src/apps/teletype.rs, src/apps/kria.rs, apps/teletype/, apps/kria/
  manifests, teletype/scenes/ + make_scenes.py. No shared files beyond the two
  new manifests (mod_inputs regenerated with PORTAMAX_WRITE_MANIFESTS).
- Status: done. Teletype: the scripting language, 8 scripts + M + I, patterns,
  DEL/S/EVERY/SKIP, scenes in the module's text format, TR/CV routed to notes or
  mod inputs. Kria: 4 tracks x 7 params with per-param loops, divisions,
  probability, directions, scales, patterns + cue, presets. Teletype reaches it
  through kria::Link (lock-free rings + atomics), like Ansible over I2C.
  Not done: Kria meta-sequencer, div-sync/cue-div, duration tie, scale fine-adjust.
- Tests: cargo test --bin portamax-sim, 1007 passed.
- Notes for the other assistant: none.

### 2026-10-05: Claude: Dexed clipping and level clicks
- Branch: claude/atlas-play
- Changed: `src/apps/dexed.rs` only. The engine's voices are summed at full scale and went past 1.0 on chords (12 notes on a plain sine patch peaked over 1.0; real patches are louder), so the device clipped them hard. Output now goes through a soft limiter (untouched below 0.6, bends smoothly to 1.0), and the master level glides across each block instead of stepping.
- Not changed: voice stealing, the engine, the bridge. The 'real syx in the repo' test now requires |sample| <= 1.0 (was 4.0).
- Tests: 2 new (chord never clips and a single note is untouched; level change doesn't click).

### 2026-10-05: Claude: Plaits plays like a normal synth; O&C licensing review
- Branch: claude/atlas-play
- Plaits: `src/apps/plaits.rs`, `src/apps/plaits_play.rs` (`Slot::age` public), `src/plaits_ffi.rs` + `vendor/bridge/plaits_bridge.cc` (new `plaits_voice_render_held`, which patches Plaits' LEVEL input; the old entry point and `plaits_smoke` are untouched). Cause of the odd feel: the voice was always run trigger-only, so its low-pass gate plucked and died away under a held key, and a second key in Mono changed pitch without a new strike. Now: Envelope row (Synth default / Pluck = the module's way), Poly default, a new key over a held one drops the gate for a frame so it strikes, and Poly loudness is a smoothed 1/sqrt(voices) instead of dividing by the count.
- Not changed: Trio, Sequencer, Bloom, Nebula, Madness, Ledger still use the plucking voice (they are triggered, not held).
- Licensing review of the O&C firmwares: recorded in `vendor/o_c/NOTICE.md`. Short version: no upstream licence files; MIT headers on most files; the maintainers say the bundle is GPLv3 *because of* files we do not ship (SH1106 driver, OC_DAC.cpp, Grids data); the unlabelled files and that claim are the residual risk. Mutable code compiled here is MIT (no AVR/GPL projects), Game_Music_Emu is LGPL (source is public), Dexed Apache-2, Orca MIT.
- Tests: 3 new Plaits (sustain vs pluck, chord loudness, new key in Mono).

### 2026-10-05: Claude: Sample Drum time-stretch
- Branch: claude/atlas-play
- Changed: `src/apps/sample_drum.rs`, new `src/apps/sample_drum_stretch.rs` (private part via `#[path]`: WSOLA, 2048-sample Hann grains, 4x overlap, +-384 sample similarity search), USER_MANUAL. Tempo Match is now Off / Stretch / Speed (Stretch keeps pitch; Speed is the old repitch); Tune still retunes a stretched sample. Presets store `tempo_mode`.
- Limits: plain WSOLA, no transient detection; fine on drums at modest ratios, slightly phasey on sustained tonal material at big ratios. Backward and loop modes use it too, untested by ear.
- Tests: 3 new (tone keeps pitch at 0.75/1/1.5x, a hit keeps its attack, voice-level stretch vs speed vs tune).

### 2026-10-05: Claude: Sample Drum slice pads and tempo match
- Branch: claude/atlas-play
- Changed: `src/apps/sample_drum.rs`, USER_MANUAL. New pad layers SLICES / SLICES 17+ (one pad per slice, via `slice_request`); Tempo Match + Loop Bars (speed-matches a loop to the shared `Clock` tempo; Auto Clock follows the project bar). Loop bars/tempo-match are guessed from the sample name's tempo and length when a sample is picked.
- Limits: matching is speed (repitch), not time-stretch -- a true stretch would be a separate piece of DSP. Auto Clock follows the project *tempo* but not its beat position/transport.
- Tests: 4 new (slice pads, tempo-in-name/bars guess, tempo-match speed, Auto Clock bar tiling for 4 vs 8 slices).

### 2026-10-05: Claude: Sequencer step expression + 808/909 drum voices
- Branch: claude/atlas-play
- Changed: `src/apps/sequencer.rs` (per-step velocity, accent, flam and chance, with a per-track accent amount, in the pattern snapshots and the step labels; the drum kind list grows to the 808/909 set; a closed hat chokes open hats), new `src/apps/sequencer_drums.rs` (private part via `#[path]`: the voices, with an honest header on what is modelled vs sampled), USER_MANUAL row.
- Flam note: there is no look-ahead scheduling, so a flam is a soft grace note on the beat and the main hit a few ms later. Block-granular like rolls (grace and hit within one audio block collapse), hence a 12 ms minimum gap.
- Tests: 8 new in sequencer + drums (velocity/accent levels, chance gating, flam scheduling, every voice audible/bounded/dies, kick pitch, hat brightness, decay knob, open vs closed + choke, clap bursts). Not heard by ear -- the voice tunings are by-ear estimates; tune them if they sound off.

### 2026-10-05: Claude: Sample Drum rework (notes, cents, envelope shapes/Relative, no clicks)
- Branch: claude/atlas-play
- Changed: `src/apps/sample_drum.rs` only (+ USER_MANUAL row). Notes now play it at pitch (the manifest already said `notes_in` but the app ignored them); Fine Tune in cents; envelope Relative range and A/D shapes (the two features the old header said were skipped); retrigger fades the old playhead out instead of cutting; opens with a sample on each channel; audio thread no longer allocates per block, takes the scope mutex with `try_lock`, and sets filter coefficients once per block.
- Checked against the Erica manual (manualslib pages 3-11). Still not done: manual slice-point editing, CV Assign, trigger delay, separate outputs per channel (it mixes both to one), SINGLE/DOUBLE projects.
- Tests: 7 new (pitch, velocity/target, release, Relative, shapes, retrigger click, cents); all 24 Sample Drum tests pass.

### 2026-10-05: Claude: O&C firmware per module (stock, Hemisphere Suite, Phazerville Suite)
- Branch: claude/atlas-play
- Changed: `build.rs` (each firmware is a shared library built from `O_C_VARIANTS`; per-variant C++ standard; replaces the four namespaced copies), `src/apps/oc.rs` + `src/apps/oc_firmware.rs` (loads a private copy of the library per module, stops/unloads safely, a **Firmware** menu row, per-variant saves `saves/oc/eeprom_<variant><n>.bin`), `vendor/o_c/host/` (shared Arduino/Teensy stand-ins), new `vendor/o_c_hemi/` and `vendor/o_c_phaz/` (NOTICE.md and PATCHES.md in each), `tools/oc_port_patches.py`, `tools/oc_sketch.py`.
- Left out on purpose: Benisphere (folded into Phazerville, unmaintained), squares-and-circles (Teensy 4 audio engines tied to hardware; redundant with the Mutable apps here), micro_Crimes (Hemisphere on a Daisy: same firmware again).
- Licensing: none of the three firmwares ships a licence file, and 41/74 files have no header; GPL files (DAC driver, SH1106 driver, Grids data/DrumMap) are not included. Flagged in each NOTICE.md; the owner should decide before redistributing.
- Tests: boot and quantize (stock), reflash a running module between firmwares, Phazerville boots and draws. Not tried by hand: using the applets on the screen.

### 2026-10-05: Claude: four O&C modules at once, each able to have its own window
- Branch: claude/atlas-play
- Changed: `build.rs` (the O&C firmware is compiled four times, each inside its own
  `namespace oc0..oc3`, since it keeps its state in globals), `vendor/o_c/host/`
  (per-instance pins/EEPROM/CV/DAC/frame table in `oc_host_core.cpp`; the part that
  defines the firmware's own class members is `oc_host_fw.cpp`, compiled per
  namespace), `src/apps/oc.rs` (instance 0-3, ids `oc`, `oc2`..`oc4`, names, saves
  `saves/oc/eeprom{,2,3,4}.bin`), `apps/oc2..4/manifest.toml`.
- Shared files touched: `src/app.rs` (three default methods on `App`: `popout`,
  `popout_key`, `popout_closed` -- a window of an app's own) and
  `examples/slint_home_live.rs` (opens, updates and closes those windows, forwarding
  the keyboard). Only the Slint shell shows them; the minifb `os.rs` path does not.
- Tests: a test boots all four firmwares together and checks each quantizes its own
  CV and that turning one's encoder changes only its screen. Not tried by eye: the
  window itself (needs the live shell).

### 2026-10-05: Claude: ten new apps from real engines (Airwindows, Braids, Stages, Peaks, Streams, SoundFont, Dexed, Chip Player, Orca, O&C)
- Branch: claude/atlas-play
- Added (each is a file in `src/apps/` + `apps/<id>/manifest.toml`, with real tests):
  `airwindows.rs` (the whole ~520-effect library through a VST2 shim),
  `braids.rs`, `stages.rs`, `peaks.rs`, `streams.rs` (Mutable, via `vendor/bridge/*_bridge.cc`),
  `soundfont.rs` (TinySoundFont), `dexed.rs` (Dexed's msfa engine; **replaces
  Cascade**: its manifest is removed, `cascade.rs` and its Slint code stay in the
  tree unused), `chip_player.rs` (Game_Music_Emu), `orca.rs` (the real Orca-c
  simulation, edited by pad, a note source on the note bus), `oc.rs` (the real
  Ornaments & Crimes firmware: `vendor/o_c`, firmware thread + host stand-in
  for the Teensy; its timer ISRs run on the audio thread).
- Shared files touched (Max decides per change, so check these): `build.rs`
  (compile helpers for C sources, per-bridge `.includes`/`.defines`, an Airwindows
  generator, the O&C build), `Cargo.toml` (+`flate2`, already in the lock, for
  `.vgz`), `.gitignore` (soundfonts/, chiptunes/), `docs/USER_MANUAL.md`.
- Vendored: `vendor/airwindows` (MIT), `vendor/dexed` (Apache-2.0 msfa only),
  `vendor/tinysoundfont` (MIT), `vendor/gme` (**LGPL-2.1**), `vendor/orca` (MIT),
  `vendor/o_c` (see its NOTICE.md: about 35 core files have no licence header; two
  GPL-3 files were left out and rewritten). Patches to vendored code are in
  `vendor/PATCHES.md`, `vendor/airwindows/PATCHES.md`, `vendor/o_c/PATCHES.md`.
  A real endless loop in the shared tides2 ramp extractor was fixed (it hung
  Stages' PLL oscillator).
- Not done: Mutable's AVR modules (Grids, Edges, Branches, Frames, Yarns) are GPL and stay out. Streams' VCA and
  filter are analogue on the module; here its gain drives a digital VCA and the filter frequency is a patchable output only.
- Status: done, but untested on hardware, and the O&C firmware runs on a thread
  shared in spirit with the audio thread like the real chip's ISRs (data races the
  firmware tolerates on one core are possible on many; none seen).
- Tests: per-app suites pass; full suite 942 of 943 passed with one
  intermittent failure in `stages::every_preset_runs_without_blowing_up` that I
  could not reproduce in 100+ runs (message now names the preset and value).

### 2026-10-04: Claude: hold-to-sweep and the one-line hint (contract steps 1-2)
- Branch: claude/atlas-play
- Changed: `src/controller_map.rs` (held D-pad ◀▶ is worth x1, x5 after 0.75 s,
  x20 after 1.75 s; ▲▼ never multiplies; `hold_multiplier`), `device_frame.slint`
  (on-screen D-pad does the same, `dpad-step`), `src/app.rs` (`App::hint()` and
  the `hints` table, with tests), `src/os.rs` and the Slint shell (a 18 px hint
  strip above the F bar; app body is now 266 px tall, was 284). Removed the old
  hard-coded "R1 SELECT / ◀▶ EDIT / R1 OPEN" lines in the list header and the
  generic panel footer.
- Not done: per-parameter detents, so apps with tiny private multipliers
  still sweep slowly even at x20. The home screen's "R1 OPEN APPLICATION" button
  text is untouched. Hint lines are not yet checked against each app's real
  behavior beyond the shared-kit tests.
- Tests: controller_map 10 pass; full suite 886 pass, 1 fail (marbles
  gate test, passes alone: load flake like Norns).

### 2026-10-04: Claude: control contract decided; PS5 profile and L1+stick built
- Branch: claude/atlas-play   Commits: see git log on the branch
- Changed: `docs/CONTROL_CONTRACT.md` (Max's decisions recorded, section 8),
  `src/play_kit.rs` (hold L1 + joystick sets the selected dial: left 0%,
  right 100%, value kept on release; the lit pad, dial, D-pad and stick all
  mean one control during a hold), `src/controller_map.rs` (new PS5
  defaults, and SELECT is tap-to-select / hold-to-reset like the device),
  the Atlas how-to (new 7.2, new PS5 appendix) and its harness.
- The PS5 layout was changed at Max's direct request: D-pad up/down browse
  and left/right change a value in every context; touchpad = SELECT; PS
  button = Home; right stick = the device joystick; L2/R2 unbound; the four
  face buttons = pads 1, 5, 9, 13. Max said "Square, circle, X and O"; I read
  the fourth as Triangle and gave them pads 1, 5, 9, 13 in the order named
  (Square, Circle, Cross, Triangle). One line each in `defaults()` to change.
- **Side effects:** Retro's A/B/X/Y are no longer on the face buttons (D-pad,
  shoulders, Options/Create still feed Retro). The saved map the Controller
  app had written was moved to `saves/controller_map.json.bak-2026-10-04`,
  otherwise it would have overridden the new defaults.
- Ownership note: `controller_map.rs` is not in the table above and git
  cannot tell which assistant wrote it (both commit as Max). An earlier entry
  of mine called it ChatGPT's area; that was an assumption, not a fact.
- Status: partial. Not built: acceleration and per-parameter detents, the
  generated hint line, confirm-by-hold, text wrap, stick scrubbing in lists,
  the contract test suite.
- Tests: kit 20, controller_map 9 and controller_setup 2 pass. Full suite: 882
  passed, 2 failed, 7 ignored: controller_setup's play-map test (it expected
  L2 on the hand sensor; fixed, now passes) and the Norns bundled-script test
  (silent "drone" under load, as logged before; it passes alone in 159 s).
- Request: none.

### 2026-10-04: Claude: Atlas how-to (PDF + screenshot harness)
- Branch: claude/atlas-play   Commits: see git log on the branch
- Changed: new `docs/atlas-howto/` (HTML source + 16 figures),
  `tools/atlas_howto_figures.py`, `examples/slint_common/atlas_howto.rs`
  (renders the app's real states through the Slint shell). Shared, minimal:
  `examples/slint_home_live.rs` (a `--render-atlas-howto DIR` flag),
  `examples/slint_common/instrument_preview.rs` (declares the module),
  `src/play_kit.rs` (moments/bindings folder can be set with
  `PORTAMAX_SAVES_DIR`, so screenshot runs never touch real saves). Atlas:
  the "Gliding to state X" message now clears once the glide ends.
- Status: done for Atlas. The PDF is built from the HTML with headless
  Chrome (`--print-to-pdf`), and the guide's Appendix C lists the control
  problems it found. It is the template for the other apps' guides.
- Tests: kit 13 and Atlas 9 pass.
- Notes for the other assistant: the same harness shape works for any app
  (copy `atlas_howto.rs`, change the states). Findings that cross into your
  area are in Appendix C (e.g. PS5 D-pad meaning differs between Home and
  play apps; Reset and F4 have no default gamepad button).
- Request: none.

### 2026-10-04: Claude: dial-free modulation (pad pressure, bind-by-wiggle, Atlas states)
- Branch: claude/atlas-play (cut from 341d3fa, my last commit; oracle-pulsar
  is behind it)   Commits: see git log on the branch
- Changed: Atlas (mine): STATES pad layer, morph glide, pressure -> ENERGY.
  **Shared files, minimal and additive**, because Max said the pads are
  pressure sensitive and asked for a dial-free design: `src/app.rs` (new
  `Input::pad_pressure`, `SIM_PAD_PRESSURE`), `src/controller.rs` (new
  `pad_pressure` atomics), `src/main.rs` (Push 2 velocity + poly aftertouch
  fill it), `src/play_kit.rs` (live routes copied from `cfg.routes`, a
  pressure route, bind-by-wiggling, `saves/<app>/routes.json`, new
  `PlayHost::kit_pads_play` with a default), `docs/PLAY_KIT.md`.
- Status: partial. Done: pressure as a control source, rebindable routes
  for every play-kit app (they inherit it, no per-app change), Atlas states
  on pads. Not done: a gesture looper (record a stick/hand move and loop it),
  per-note pressure inside the synthesis engine (pressure is currently the
  firmest pad, applied to one control), an on-screen marker for the pressure
  route beyond the status line, and the real pad driver filling
  `ControllerState::pad_pressure` (the simulator and a Push 2 do).
- Tests: kit 13 pass (6 new), Atlas 9 pass (3 new); full suite 873 passed, 0 failed, 7 ignored. One
  existing kit test changed: it set `k.cfg.routes`, which is now only the
  starting point, so it sets `k.routes` instead.
- Notes for the other assistant: nothing for the Kids apps to change. The
  other ~40 play-kit apps can now be rebound by wiggling; their own
  `KitConfig` routes still apply until the player rebinds. `CLAUDE.md` still
  says "two knobs/encoders" (shared file, left alone for Max).
- Request: none.

### 2026-10-04: Claude: device clock, Chop, Looper, Tempo, Choir, Sorter, Ledger sync
- Branch: oracle-pulsar (synced to Max's Mac by patch)   Commits: e456175..e8122c1, cf5cfdc, 8164f37 (Mac hashes)
- Changed: new apps chop, tempo, looper, choir, sorter (+ assets/npu/sorter_*,
  tools/npu/train_sorter.py, features.py); new src/clock.rs. Shared, edited
  before this file existed: src/audio.rs (MixBus advances the clock after
  each block), src/main.rs + every examples/*.rs that includes audio.rs
  (`mod clock`), src/midi_devices.rs (MIDI clock out), live_midi.rs/main.rs
  MIDI input (clock in), docs/USER_MANUAL.md, .gitignore (all of samples/).
  Also session.rs, skins.rs, ledger.rs (follow the clock), hum.rs and
  ai_input.rs (a few items made pub for Choir), kids_kit.rs (see below).
- Status: done.
- Tests: full cargo test --bin portamax-sim with samples/ present: all pass
  (the 12 sample_drum/sequencer failures were only the missing samples).
  Sorter also checked on Max's real library (opt-in `sorter_report` test).
- Notes for the other assistant: kids_kit::Sound gained `set_follow` /
  `following` / `clock()` / `set_volume`, and its processor can follow the
  device clock. Default is OFF, so the Kids apps keep their own tempo;
  `Sound::new` now takes the clock internally (no signature change).
  Clock::shared() is per-test-isolated under cfg(test).

### 2026-10-04: ChatGPT: friendly illustrated cat and voice mascots
- Branch: gpt/kids-ux   Commits: d10fca6
- Changed: src/apps/copy_cat.rs, src/apps/monster_mic.rs,
  src/apps/kids_kit.rs (new opt-in artwork renderer), AGENTS.md.
- Status: done for this character revision. Four illustrated kitten poses,
  eight original voice portraits, live voice bob/meter feedback. The original
  concept preview was not present in the retrieved chat or project sources;
  precise matching of the full ten-app set still needs that reference image.
- Tests: cargo test --bin portamax-sim apps::copy_cat (4 passed),
  apps::monster_mic (5 passed), apps::kids_kit (7 passed), repeated after
  final atlas gutter correction. Full suite: 861 passed, 6 ignored, 1 failed:
  untouched Norns every_bundled_script_runs_and_plays reported silent tidepool.
  Rechecked with PORTAMAX_NORNS_ONLY=tidepool: passed (1 test).
  Reviewed all twelve illustrated states at 640x360; permanent tests verify
  matching Slint frames, PNG validation, cell selection and alpha blending.
- Notes for the other assistant: no controls, DSP, manifests, dependencies
  or docs changed. Artwork is embedded in its app as base64 PNG to stay within
  the authorized files; decoded once on the UI thread with OnceLock. Approx.
  0.55-0.98 ms per cached desktop draw; no MCU performance claim. Original
  artwork, prompts and screenshots are saved in this chat's workspace.
  Shared existing helpers and all other app screens are unchanged.

### 2026-10-04: ChatGPT: cohesive, readable screens for the ten Kids apps
- Branch: gpt/kids-ux (from oracle-pulsar)   Commits: b2d4b81
- Changed: draw() in critter_choir, rainbow_bells, copy_cat, bug_beats,
  monster_mic, beat_lab, chord_garden, sound_detective, ear_quest, music_code;
  src/apps/kids_kit.rs drawing helpers; AGENTS.md locks and log.
- Status: done. Warm paper/teal chrome, tactile cards, selection outlines,
  pad-location labels, readable recording/empty-clip states and shorter hints.
- Tests: cargo test --bin portamax-sim apps::<id> for all ten (40 passed),
  apps::kids_kit (5 passed), full cargo test --bin portamax-sim
  (858 passed, 6 ignored, 0 failed). Temporary visual harness checked idle
  and active renders and identical draw()/screen_extra pixels for all ten;
  reviewed 640x360 screenshots, then removed the harness.
- Notes for the other assistant: controls/audio unchanged; manifests and
  docs unchanged. New kids_header/kids_footer/card helpers are opt-in;
  existing generic helpers and the shared sound engine are unchanged.
  No Bluey assets, branding or characters were added.

### 2026-10-04: Claude: ten synthesis blocks ported from Cardinal modules
- Branch: cardinal-ports, fast-forwarded into oracle-pulsar   Commits: daebfce
- Changed: src/synthesis/blocks.rs, src/synthesis/blocks/ports.rs (new),
  src/synthesis/engine.rs (delay_seconds), docs/SYNTH_PLATFORM.md (shared
  area, edited before this file existed: a block table only)
- Status: done. New blocks: plateau, spring, phaser, freqshift, rotary, tape,
  comp, kick, walk, eq. Each one is written from the published technique,
  not the GPL module source, so the repo stays MIT.
- Tests: cargo test --bin portamax-sim, all pass. The sample_drum and
  sequencer tests need samples/ to be present.
- Notes for the other assistant: Oracle's prompt now offers these ten blocks,
  since SPECS generates it. Their cost entries are conservative; re-measure
  with block_costs on an idle machine.

### 2026-10-04: Claude: created AGENTS.md
- Branch: oracle-pulsar (uncommitted)
- Changed: AGENTS.md (new); one pointer line added to the top of CLAUDE.md
- Status: done. Max to confirm the ownership table.
- Notes for the other assistant: please read sections 1–3 before your next task.

## 6. Before you finish

1. Run the tests that touch your area. At minimum, run
   `cargo test --bin portamax-sim`. If you changed presets or blocks, also run
   `cargo test --bin portamax-sim every_factory_preset_compiles_plays_and_morphs`.
2. Commit with a descriptive message that says *why*, not just *what*.
3. Add your Work log entry, and clear your lock in section 4.
4. Tell Max the branch name and anything the other assistant needs to know.
