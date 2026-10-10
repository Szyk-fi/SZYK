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
| — | none | — | — |

## 5. Work log

### 2026-10-09: Claude: Rev2 cutoff shift removed again, measured scale restored
- Branch: main
- Changed: src/apps/prophet/dsp.rs (CUTOFF_SHIFT back to 0, anchor test back to
  105 = 440 Hz), docs/PROPHET.md. Once FILTER_LEAK was in, the octave shift gained
  under 1.5 dB mean error on the dry set and nothing on Sequential's factory demos,
  and made cutoffs above 120 6 dB too bright. It had been compensating for the
  missing leak. Forum measurements (creativespiral) say 105 = 440 Hz, so that stays.
- Status: done. Tests: full suite.
- Notes for the other assistant: the +12 from the earlier entry below is gone.

### 2026-10-09: Claude: Rev2 filter leakage from 110 dry recordings
- Branch: claude/rev2-teletype-clean (merged to main)
- Changed: src/apps/prophet/dsp.rs (FILTER_LEAK 2% low-passed at 1 kHz; ignored
  dev test takes REV2_DRY, REV2_STEP, REV2_SYX, REV2_MAP), docs/PROPHET.md,
  tools/soundcloud_playlist.sh. Also checked against ten of Sequential's
  official factory demos (factory .syx fetched from sequential.com, not in repo).
  Max's SoundCloud REVField dry set (110 tracks) vs the app: sawtooth/pulse
  patches were 10-25 dB too dark above 1 kHz. The ideal filter's stopband is
  too deep; a leak around it cut mean band error 18.0 -> 13.1 dB.
- Status: partial. Cutoff >120 too bright (+9 dB); filter-envelope sweep of
  low-cutoff patches may be under-scaled; 8 worst clips listed in PROPHET.md.
- Tests: prophet tests pass; full suite run before commit.
- Notes for the other assistant: Rev2 is yours; please review.

### 2026-10-09: Claude: Rev2 filter cutoff calibrated against recordings
- Branch: claude/instrument-audit-teletype
- Changed: src/apps/prophet/dsp.rs (CUTOFF_SHIFT = +12 semitones; new ignored
  dev test reference_spectral_match), docs/PROPHET.md. Max supplied clips of
  REVField programs 1-10 played on a real Rev2. Level-matched third-octave
  spectra showed the app 5-16 dB too dark above 600 Hz; +12..+15 st flattened
  the median in 8 of 9 usable clips (program 9 got worse).
- Status: partial. Audio-mod depth, sync direction, programs 1/5/10 still dark.
  This overrides the earlier CreativeSpiral anchor (105 = 440 Hz); ChatGPT owns
  this app, please review.
- Tests: prophet tests pass (anchor test now expects 880 Hz).

### 2026-10-09: Claude: Teletype scenes draw their own grids; Grid Pads/Knobs routing overview; Rev2 survey
- Branch: claude/instrument-audit-teletype
- Changed: teletype/make_scenes.py + scenes 01-05 (each now has G ops: faders,
  buttons, playheads; the grid is 16 x 8 so everything sits in rows 0-7),
  src/apps/teletype.rs (hint for G scenes), src/apps/grid_pads.rs and
  arc_knobs.rs (one routing row per pad/knob, "App > Input", knob 2 steps
  through every input; was two rows per output), src/registry.rs test,
  docs/USER_MANUAL.md 5.3.
- Rev2: no code change. Survey of the 440 supplied programs: the four
  REVField bank files are identical copies of 110 programs; every parameter is
  in range; offset 20 (Pitch Bend Range) is 7 in 436 of 440 (hardware default is
  2, so check this offset); with C4 held for 6 s the median program renders
  0.022 RMS against 0.15 for Init, and 22% under 0.005 (sustain 0 plucks, closed
  cutoff with Key Amount 0). Needs a named patch to compare against hardware.
- Status: done except the Rev2 question.
- Tests: full suite.
- Notes for the other assistant: Rev2 is yours; please look at the pitch bend
  offset and the quiet-patch loudness.

### 2026-10-09: Claude: instrument/Plays audit, read-only instruments editable, Teletype links on screen
- Branch: gpt/bebot-1791467933 (as found)   Commits: none yet
- Audit (new test registry::audit_every_source_and_instrument): all 33 instruments
  publish settings through the note bus; every flat-menu source (Chordsmith,
  Constellation, Dream, Hum, Kria, Marbles, Mutant, Nebula, O&C 1-4, Orbit, Orca,
  Swarm, Teletype, Turing Machine) leads with Plays and lists them under it.
  Gaps found: Choir, Chop, Skins, Timbre Map and Rev2 listed settings read-only
  (fixed: each now answers adjust_setting); Marbles had Plays at row 17 (fixed:
  new Module::extras_first in mi_kit). Not fixed: Ledger lists no settings under
  Plays (its menu is the kit control list plus the tracker, no place to insert
  rows). Plays sits inside a group by design in Bloom, Madness, Norns, Sequencer,
  Session. No notes_in: rainbow_bells, critter_choir, chip_player (candidates);
  queen_of_pentacles and pams are CV/gate sources with no note output.
- Teletype: it already routed TR to instruments (Plays row) and had grid support;
  the gap was visibility. Menu and side panel now show `TR plays: ...` and a
  `Grid: ...` status line; USER_MANUAL 5.3 explains the links. New tests: notes
  reach the instrument over the bus; grid status text.
- Changed: src/apps/{choir,chop,skins,timbre_map,prophet/mod,marbles,mi_kit,teletype}.rs,
  src/registry.rs, docs/USER_MANUAL.md, AGENTS.md.
- Status: done except Ledger.
- Notes for the other assistant: prophet/mod.rs and bebot area touched only to add
  adjust_setting to Rev2 (ChatGPT's app); please review.

### 2026-10-08: ChatGPT: prepare authorized Rev2 merge and publish
- Branch: gpt/rev2-parameter-ui-audit
- Changed: tools/apply-rev2-audit.command adds --publish; --run remains optional.
- Status: blocked executing merge. Max explicitly requested merging. GitHub main
  still 977d8c3; integration branch creation returns HTTP 403. Local git fetch
  cannot write .git/FETCH_HEAD (Operation not permitted). No main files changed.
- Tests: bash -n and git diff --check pass. DSP/UI source unchanged from the
  verified Rev2 audit. One Terminal command now applies and publishes main.
- Notes for the other assistant: Max's imported patches and modified Eurorack
  worktree remain untouched. No approval was withheld; execution access is blocked.

### 2026-10-08: ChatGPT: Rev2 wire/DSP audit and real Slint panel
- Branch: gpt/rev2-parameter-ui-audit   Commits: this changeset
- Changed: src/apps/prophet/, src/manifest.rs, src/app.rs, live Slint example,
  instrument preview, docs/PROPHET.md, detailed audit/parameter CSV/screenshots,
  tools/apply-rev2-audit.command.
- Status: audit fixes and Slint panel done on the isolated branch; installation
  and publishing blocked by checkout write restrictions and GitHub HTTP 403.
  Hardware fidelity remains partial; see docs/REV2_AUDIT_2026-10-08.md.
- Tests: final broad suite 1201 passed, 0 failed, 14 ignored, 3 excluded (two UDP
  binds forbidden by sandbox; previously documented stalled Norns bundled-script
  test). All 54 focused Rev2 tests pass in that run; optional external Edisyn
  init import/export/audio test separately passes. Atlas factory compile/play/
  morph test included. Live Slint renders all 16 sections, verifies mouse button/
  drag edits, layer/page/tab controls, all sequence ranges/tracks and key release.
  440 imported patches plus all 16 original starts render finite bounded audio.
  Apply script passes bash -n; git diff --check clean.
- Notes for the other assistant: latest main 977d8c3 verified before edits and
  again at completion. Stack=1/Split=2; poly velocity <=127 Reset,128 Rest,
 129..255 velocity1..127; native bytes never rewritten on import. Cutoff and VCA
  attack use published measured anchors; slop depth/taper remain approximate.
  No shared synthesis engine, dependency or registry edits. Main's dirty
  Eurorack submodule and imported patches preserved. Use apply script with --run
  to fast-forward the unchanged base and launch the actual Slint UI.


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

### 2026-10-08: ChatGPT: Bebot-inspired mouse-drag instrument
- Branch: gpt/bebot
- Changed: new src/apps/bebot/, apps/bebot/manifest.toml, docs/BEBOT.md;
  examples/slint_common/bebot_panel.slint + bebot_preview.rs;
  examples/slint_home_live.rs imports/selects the dedicated panel and exposes
  --render-bebot. No existing DSP, buses, dependencies or registry edited.
- Status: implementation done; remote publication blocked. Max requested merging
  to main, but GitHub writes return 403 and automatic approval review requires
  explicit confirmation of the destination Szyk-fi/SZYK before a push.
- Tests: 13 Bebot behavior tests pass in the actual Portamax binary. Four extra
  tests importing the actual DSP passed (pitch/release/rates, slow tune, extreme
  effects across 8-192 kHz, invalid rate). Live Slint shell builds; actual mouse
  press/drag/release/right-click and settings interaction verified by
  --render-bebot; both screenshots visually reviewed. Broad suite: 1187 passed,
  19 failed because this checkout has no samples/, 14 ignored, one previously
  stalled Norns bundled-script test excluded. Launcher/manifest contracts pass.
- Notes for the other assistant: independent implementation and original robot;
  not Normalware's engine/artwork and not exact sonic parity. One mouse contact
  plus seven stable pad/MIDI voices; eight new presets, one saved user preset.
  No four-contact touchscreen driver, custom note masks or preset folders yet.

### 2026-10-07: ChatGPT: Rev2 sound and modulation audit
- Branch: gpt/rev2-sound-audit
- Changed: src/apps/prophet/dsp.rs, docs/PROPHET.md, AGENTS.md.
- Status: DSP corrections implemented; hardware sound calibration remains
  incomplete. Publication uses Max's authenticated Terminal because this agent
  cannot retrieve the GitHub keyring credential.
- Tests: 20 DSP behavior tests passed in a harness importing actual modules;
  final Cargo suite: 1193 passed, 0 failed, 14 ignored, excluding the previously
  stalled unrelated Norns bundled-script test. Rendered all 16 original starting
  points and 440 supplied native programs, single notes and four-note chords
  at 48 kHz/velocity 100: imported peaks <=0.745, no samples above 0.95.
  Unmodulated A4 = 440 Hz; slow independent drift, routing depth/polarity,
  free LFOs, key sync, filter body/tracking, DC/alias reduction, layer seed
  independence, chorus wet-only mix, reverb stereo/decay and overload verified.
- Notes for the other assistant: standalone Rev2 DSP only. Preset bytes,
  importer, dependencies and shared buses are unchanged. Restores filter body,
  corrects square sub/audio-mod source/aux repeat/pan, replaces the sparse reverb
  with an eight-line FDN. Pitch routes follow published hardware measurements;
  original curves/effects are still uncalibrated. No listening A/B against
  hardware was performed. Do not call this complete Rev2 fidelity.

### 2026-10-07: ChatGPT: correct low Rev2 output level
- Branch: gpt/rev2-output-level
- Changed: src/apps/prophet/dsp.rs, docs/PROPHET.md, AGENTS.md.
- Status: output fix implemented; publication requires Max's authenticated
  Terminal because this agent cannot retrieve the GitHub keyring credential.
  Adds 12 dB of makeup after effects, before the existing mixer/soft ceiling.
  Internal filter/effect drive and native patch volume bytes stay unchanged.
- Tests: 31 Rev2 tests passed (2 optional tests ignored). Broad Cargo suite:
  1179 passed, 0 failed, 14 ignored; previously stalled Norns bundled-script
  test excluded. Six DSP behavior tests also passed in a lightweight harness
  using the actual DSP, patch, preset, tuning, atomic and clock modules.
  Measured all 16 starting points plus 440 imported programs at velocity 100;
  imported single-note/chord peaks <=0.707, no samples above 0.95. Init single
  peak increases from 0.0567 to 0.2231; dense/stacked chords remain bounded.
- Notes for the other assistant: standalone Rev2 change; no shared audio/mixer
  code or dependencies touched. Full hardware parity remains incomplete.

### 2026-10-07: ChatGPT: Rev2-compatible patch player; exact clone still in progress
- Branch: gpt/rev2-instrument
- Changed: src/apps/prophet/, apps/prophet/manifest.toml, docs/PROPHET.md,
  docs/USER_MANUAL.md. Replaces the previously unmerged Prophet-style prototype.
- Status: partial. Native Rev2 SysEx import/export and two-layer playback work.
  All public parameter offsets are retained; chord-memory bytes stay opaque.
  See docs/PROPHET.md for remaining recording, MIDI, clock and fidelity gaps.
  Max explicitly authorized merging this current partial implementation on
  2026-10-07 after reviewing the remaining gaps. Earlier publishing was blocked
  by missing write access. The CLI configuration was refreshed, but this agent
  session still cannot retrieve the credential; connector writes return 403.
  An authenticated Terminal merge script is prepared. Do not call this full parity.
  Hardware chord before/after dumps and sound references
  are needed for exact verification. Do not label this completed full parity.
- Tests: 29 focused passed, 2 optional review/reference tests ignored by default;
  both optional tests separately passed. Edisyn init dump round-trips byte-for-byte
  and plays. Manifest contract passes; slint_home_live cargo check passes.
  Broad suite excluding every_bundled_script_runs_and_plays: 1177 passed,
  0 failed, 14 ignored. Initial full run was interrupted after that untouched
  Norns test did not finish; a discovered manifest mismatch was fixed and retested.
- Notes for the other assistant: standalone app only; no shared engine, buses,
  dependencies or registry changes. App id prophet; display/audio/mixer Rev2.
  Current remote-main Blaster sub work is preserved. No hardware presets committed.

### 2026-10-07: Claude: Blaster: sub oscillator
- Branch: claude/blaster-sub
- Changed: src/apps/blaster/ (params: new "Sub" group inserted before Size,
  later groups renumbered; four params appended so existing saved slots keep
  their indices), voice.rs, presets.rs, mod.rs (the play view's Chirp speed and
  Crush bits dials became Sub (charge) and Sub (blast)), docs/USER_MANUAL.md
  (3.10). A sine/triangle/square sub one or two octaves below the tone, separate
  levels for the charge and the blast; it follows the climb and the sweep and
  sounds even with Tone level 0. Off by default; on in six bass-heavy characters.
- Status: done.
- Tests: 3 new (sub an octave below and -2 oct, wave harmonics, off by default;
  blast sub follows the sweep with a noise-only blast; sub fattens the charge
  of the first character without touching its blast).
- Notes for the other assistant: none.

### 2026-10-07: Claude: New app: Blaster (hold to charge, release to fire)
- Branch: claude/blaster
- Changed: new src/apps/blaster/ (params, store, voice, engine, presets, mod,
  tests) + apps/blaster/manifest.toml, docs/USER_MANUAL.md (§3.10). Reuses
  Hydra's Def/Kind helpers and DSP primitives (apps/hydra/params.rs, dsp.rs,
  store.rs conversions); does not change them. Eight voices, each a state
  machine: Charging (climbing, quickening chirps, a flutter at full) then
  Blasting (pitch sweep, body oscillator, noise burst, size scaling, crush,
  echo). Note-on charges and note-off fires, so it plays from the pads and from
  any app's sequencer over the note bus. 27 characters, Randomize/Mutate, 8
  user slots, 3 modulation inputs.
- Status: done. Rendering every character to WAV is a test:
  `BLASTER_WRITE_WAVS=dir cargo test --bin portamax-sim render_demo_wavs`.
- Tests: 25 blaster tests (incl. the Reed spectrum, counted chirps landing on
  the held pitch, ripple, the first character vs its recording; charge climbs, chirps speed up, blast falls and
  ends, size scales length and volume, fire at full, key follow, polyphony and
  stealing, every character and 20 random blasts charge/fire/end, loudness).
- Notes for the other assistant: none.

### 2026-10-07: Claude: Fix: O&C firmware could not load on Linux
- Branch: claude/bloom-audio-engines
- Changed: src/apps/oc_firmware.rs. `RTLD_LOCAL` was 4 (the macOS value); on
  Linux 4 is RTLD_NOLOAD, so `dlopen` returned null with no message and every
  O&C firmware test failed with "dlopen failed". Now 0 on Linux. macOS is
  unchanged.
- Also: vendor/o_c_phaz/fw/applets/EuclidX.h divided by (length + padding) = 0 at
  boot. ARM (the module, Apple silicon) defines that as 0; x86 traps it (SIGFPE),
  which killed the whole test process once the loader worked. Guarded in the source.
- Status: done. All apps::oc tests pass on Linux.
- Tests: apps::oc.
- Notes for the other assistant: none.

### 2026-10-07: Claude: Bloom: shapes can gate any audio source
- Branch: claude/bloom-audio-engines
- Changed: src/apps/bloom.rs, docs/USER_MANUAL.md. Each shape gets a Gate
  audio row (Off or any audio-bus output except Bloom's own) and a Gate level;
  the shape's notes open that source (4 ms attack, release follows Decay) and
  it is mixed in after the voice-count headroom division. The audio thread
  uses `try_lock` on the source buffer, so a busy source counts as silent.
- Status: done. Instruments (note bus) and audio sources (gate) are separate
  choices per shape; both can be used at once.
- Tests: 3 new bloom tests (choices skip Bloom's own output, gate opens and
  closes with the notes and scales with level, missing source is silent).
- Notes for the other assistant: none.

### 2026-10-07: Claude: New app: Now Playing (overview of sources, instruments, levels)
- Branch: claude/now-playing
- Changed: new src/apps/now_playing.rs + apps/now_playing/manifest.toml,
  docs/USER_MANUAL.md (§2). Lists every note source with the instrument it
  plays (lit while that instrument sounds), knob 2 re-routes it (note bus
  `step_source`), press lists the instrument's own settings through the
  instrument-settings bridge; second section lists channels that are making
  sound with a live level and moves their Mixer fader. Reads/writes only the
  existing buses; no other file touched.
- Status: done.
- Tests: 7 now_playing tests (routing, activity light, settings expand and
  edit, sounding list and fader, empty world, draw).
- Notes for the other assistant: none.

### 2026-10-07: Claude: Hydra playability: chords, Hold, macros, 250 presets in folders
- Branch: claude/hydra-playability
- Changed: src/apps/hydra/ only (+ docs/USER_MANUAL.md §3.9). Engine: Chord
  (fixed shapes or built from the pad scale), Hold (latch), macro
  destinations (each macro moves two parameters in their own scale), per-key
  chord memory so letting go stops exactly what was started. Play view: 16
  controls with a Page selector (Osc/Filter/Env/Mod/FX/Play), Folder + Preset
  browsers, four macros named by the sound. Library: 257 factory presets in 16
  folders (presets/<folder>.rs), loudness table levels.rs (written by
  `HYDRA_WRITE_LEVELS=1 cargo test --bin portamax-sim calibrate_levels`),
  user presets saved to saves/hydra/presets/<Folder>/*.json, favorites,
  Morph. Level now 0-200%. Pad layouts (scale / chromatic / fourths).
- Status: done. Not auditioned by ear (no audio device here); presets are
  designed from the DSP's parameter meanings and levelled by measurement.
- Tests: 75 hydra tests (chords, Hold, macros, pages, folders, morph, saved
  and dropped-in presets, every preset finite and levelled).
- Notes for the other assistant: none.

### 2026-10-06: Claude: New app: Hydra, an all-in-one hybrid synth
- Branch: claude/hydra
- Changed: new src/apps/hydra/ (params, store, dsp, tables, voice, fx, engine,
  presets, mod, tests) and apps/hydra/manifest.toml. 16 voices; 3 oscillators
  each Analog (PolyBLEP/BLAMP, unison to 7) / Wavetable (6 banks x 16 frames,
  inverse-FFT band-limited) / FM / Karplus-Strong Pluck / Noise; sub, ring, cross-mod,
  hard sync; two filters (TPT SVF modes, ZDF ladder) in series or parallel; 3 ADSRs,
  2 LFOs, 12-slot mod matrix, 4 macros; drive/chorus/phaser/delay/FDN reverb;
  arpeggiator; mono/legato/glide; 28 factory presets and 16 user slots
  (saves/hydra/). One params! table (151 params) drives the menu, play dials,
  Moments, settings bridge and presets. Plays through the note bus, mixer and
  ModBus (14 CV inputs) like any instrument; no other file touched.
- Status: done. Real-time-safe by design (atomics, try_lock, pre-sized buffers).
- Tests: 48 hydra tests pass (pitch by autocorrelation for every oscillator type,
  alias measurement vs naive, filters, FX, app-level pad/preset/Moments/arp/steal).
- Notes for the other assistant: none. Dialogue (yours) left untouched.

### 2026-10-06: Claude: Bloom: each shape picks its own engine
- Branch: claude/picker-and-cleanup
- Changed: src/apps/bloom.rs, apps/bloom/manifest.toml, src/note_bus.rs
  (NoteRoute::set), src/registry.rs (test). Bloom had one note route for all
  eight shapes and a Plaits engine at the bottom of every shape. Now each shape
  has its own route ("Bloom Shape 1".."8" on the note bus) and an Engine row
  right under Pattern: Own sound (the Plaits voice, with its Voice/Harmonics/
  Timbre/Decay rows listed under it), None, or any instrument, whose own
  settings (patch, plugin...) list there through the instrument-settings
  bridge; for an instrument the Plaits rows give way to its settings and Decay
  becomes the note length. The top-level Plays row sets every shape at once.
- Status: done. Not built: a "now playing" overview app.
- Tests: each_shape_picks_its_engine_under_pattern; full suite: only the 25
  environmental failures.
- Notes for the other assistant: anything that named the note source "Bloom"
  now needs "Bloom Shape N".

### 2026-10-06: Claude: Grid and Teletype: the grid is playable and says how
- Branch: claude/picker-and-cleanup
- Why it was dark: an app joined the grid only when it was built, and apps are
  built lazily, so Teletype was not on the Grid app's list until opened (and
  the first app built, e.g. Kria, held the focus); and Teletype's default
  scenes have no G ops, so even with focus it drew nothing.
- Changed: manifest field `grid_client` (kria, teletype, grid_pads, norns),
  declared at startup (Grid::declare, shared area src/apps/grid_kit.rs +
  src/registry.rs + src/app_runtime.rs); handing the grid to an unbuilt app
  builds it (LazyApp::with_grid, Grid::focus_owner). Grid::set_hint plus a line
  under the Grid screen saying how the focused app uses the grid. Teletype with
  a scene that has no G ops now shows and runs the module's script buttons on
  the top row (keys 1-8 = S1-S8, 10 = M, 11 = I, lit while running); scenes
  with G ops draw the grid themselves as before. docs/USER_MANUAL.md section
  5.3 (Grid, Teletype, Kria; the manual had nothing on them),
  docs/ADDING_AN_APP.md.
- Status: done in the sim; not tried on a real grid.
- Tests: handing_the_grid_to_an_unopened_teletype_builds_it_and_lights_the_grid;
  full suite: only the 25 environmental failures.
- Notes for the other assistant: an app that plays the grid should add
  `grid_client = "<name it registers under>"` to its manifest.

### 2026-10-06: Claude: instrument settings listed under a source's Plays row
- Branch: claude/picker-and-cleanup
- Changed: new shared mechanism. src/app.rs: `Setting`, and App gains
  `instrument_settings()` / `adjust_setting()` (play-kit apps and the MI kit
  answer from their controls via play_kit::settings_of/adjust_in; any other
  app falls back to its menu rows, read-only: choir, chop, skins, timbre_map).
  src/note_bus.rs: a settings port per instrument slot (shared area: a new
  field and methods, nothing existing changed), `NoteRoute::settings()` and
  `NoteRoute::adjust()`. src/app_runtime.rs: LazyApp publishes the instrument's
  settings while a source is asking and applies the edits queued for it.
  Source menus that now list them under Plays: Bloom, Madness, Nebula, Turing
  Machine, Marbles, Norns, Hum, Chordsmith, Session (SETUP), Collection synths,
  Sequencer (instrument tracks), Kria, Orca, Teletype, O&C.
- Status: done except Ledger (its menu is the kit's own control list, so there
  is nowhere to insert rows; Plays is there but without the settings) and
  Dialogue (not mine to edit now).
- Tests: registry tests a_source_lists_and_edits_the_settings_of_the_instrument_it_plays
  and a_sources_menu_lists_the_instruments_settings_under_plays; full suite has
  only the 25 environmental failures.
- Notes for the other assistant: a source calls `note_route.settings()` every
  frame it draws the rows (asking is what keeps the instrument publishing); to
  list them in a new source, add rows after Plays from that call and send edits
  with `note_route.adjust(i, delta)`.

### 2026-10-06: Claude: Plays first in every note source's menu, orphan apps, warnings
- Branch: claude/picker-and-cleanup (on top of claude/dropin-cleanup)
- Changed: Plays is now the first row in Kria, Teletype, Orca, O&C (constants
  renumbered), Collection synths, Hum, Turing Machine, Nebula (a top-level
  leaf), Session (SETUP) and Ledger (new Plays row: its tracks had routes but
  no row to change them) and Norns (new, first row of PARAMS; it had no
  picker). Sequencer keeps Plays first inside each track's group. Cascade
  removed (replaced by Dexed); Synth reinstalled (apps/synth, the manual
  documents it); Analyzer's `create` removed (it is Visualizer's helper).
  build.rs adds -include cstdio/cstring/cstdlib on Linux. 112 compiler
  warnings down to 0 (`#[allow(dead_code)]` where only the Slint examples use
  an item). tools/sync_manifests.sh fills in mod_inputs.
- Status: done, except the instrument's own settings listed under the Plays
  row: that needs a shared mechanism (ModBus inputs by owner) and is not built.
- Tests: cargo test --bin portamax-sim: 25 failed, all environmental (19 need
  samples/; 6 in apps::oc: dlopen of the firmware library fails here with no
  error text, undiagnosed, so the O&C renumber is only covered by the tests
  that do not boot the firmware).
- Notes for the other assistant: menu row numbers moved in those apps; any
  script or doc that named a row by position needs checking.

### 2026-10-06: Claude: drop-in app contract checked, docs and build fixed
- Branch: claude/dropin-cleanup
- Changed: docs/ADDING_AN_APP.md (rewritten: nothing central to edit, the
  manifest table says what each field gives an app), CLAUDE.md and
  docs/app-independence.md (stale: told agents to leave registry.rs/mod.rs and
  the manifest to the owner, and quoted old app/test counts), src/registry.rs
  (new test every_installed_app_is_wired_up_and_runs: every manifest has code,
  unique ids and names, notes_in/note_outputs really declared, every app can be
  entered, ticked, drawn and run with finite audio), build.rs
  (apply_vendor_patches: the Tides ramp-extractor fix from vendor/PATCHES.md,
  which lives in an upstream submodule and so was missing from fresh checkouts;
  without it Stages' every_preset_runs_without_blowing_up spins forever),
  vendor/PATCHES.md.
- Status: done. The drop-in promise already held (build.rs generates the module
  list and factory table; the manifest installs the app).
- Tests: cargo test --bin portamax-sim: 1031 passed, 25 failed. 19 need
  samples/ (sample_drum, sequencer); 6 in apps::oc ("the firmware was started")
  fail in this Linux container, not diagnosed.
- Notes for the other assistant: src/apps/{cascade,synth,analyzer}.rs have a
  `create` but no manifest (cascade was uninstalled for Dexed on purpose); I
  left them. On newer GCC the eurorack C++ needs CXXFLAGS="-include cstdio".

### 2026-10-06: Claude: grid and arc pieces after monome-community/collected
- Branch: claude/teletype
- Changed: new bundled norns scripts in assets/norns/code (all new code,
  each header credits the app it's after): bouncers (Rebound), automata
  (ricochet / Otomata), plinko (Plinkonome), charge (skr), isogrid
  (mabalhabla), strumharp (autoharp), quickhands (react), blocks (Tetris)
  for the grid; shoals (Shoal) and scrubber (plates) for the arc. New apps
  src/apps/arc_knobs.rs (knobs/quadknobs: 16 arc knobs in 4 banks routed to
  mod inputs) and src/apps/grid_pads.rs (pad: 4x4 XY pads routed to mod
  inputs), with manifests. src/apps/norns/mod.rs: a test that plays every
  grid/arc script from its hardware at two sizes, and an ignored screenshot
  test (PORTAMAX_NORNS_GRID_SHOT).
- Status: done. The collected repo is mostly Max/M4L patches with no
  licence, so nothing was copied: these are re-made from the descriptions.
  Left out: Live/Max utilities (derp, monomebridge, quadrants...), video and
  webcam pieces, and the sample players (mlr, mash, tml, wedjat...), which
  need sample loading into softcut first; theremin needs a sustaining
  engine (only PolyPerc exists).
- Tests: cargo test --bin portamax-sim -- norns arc_knobs grid_pads.

### 2026-10-06: Claude: the arc (on-screen + real monome arc), norns arc API
- Branch: claude/teletype
- Changed: new src/apps/arc_kit.rs (shared arc hub like grid_kit: 2 or 4
  rings of 64 LEDs, turns/pushes to the focused app, draw helpers), new
  src/apps/arc.rs + apps/arc/manifest.toml (Arc app: drag round a ring to
  turn it, 1024 steps a turn; click its middle to push; knobs and pads too),
  src/apps/grid_kit.rs (serialosc client also attaches an arc: /enc/delta,
  /enc/key in, /ring/map out; mext gains the query answer, encoder 0x50,
  push 0x51/0x52 and ring map 0x92; the serial driver tells an arc from a
  grid by its encoder count in the query answer and can hold one of each),
  src/apps/norns/{host.rs,mod.rs,lua/prelude.lua} (arc.connect, a.delta,
  a.key, a:led, a:all, a:segment, a:refresh), new bundled norns script
  assets/norns/code/arcarp.
- Status: done in the sim; not tried on a real arc (none here). The query
  answer layout (0x00 subsystem count, encoders = 5) is from libmonome's
  headers; if an arc ever answers differently, `serial::probe` is where to look.
- Tests: cargo test --bin portamax-sim -- arc grid_kit norns.

### 2026-10-06: Claude: Teletype's grid ops (G.*) on the shared grid
- Branch: claude/teletype
- Changed: new src/apps/teletype_grid.rs (the grid integration: 256 buttons,
  64 faders of all 8 types with hold-repeat and slides, 64 groups, 8 XY
  pads, the LED layer, G.DIM/G.ROTATE, rendering), src/apps/teletype.rs (65
  G ops in the op table, GRID and GRID+ pad pages before TRACKER, grid keys
  in through the edit queue and a 16 x 16 picture out, Teletype registers as
  a grid client and a scene using G ops takes the grid), new bundled scene
  teletype/scenes/06_grid_steps.txt (from make_scenes.py).
- Status: done. Behaviour matched to the firmware's (read for behaviour
  only, GPL); grid control mode and the on-screen grid visualiser are not
  done.
- Tests: cargo test --bin portamax-sim -- teletype (8 in teletype_grid, one
  app test playing the grid scene from the shared grid).

### 2026-10-06: Claude: norns `grid` API on the shared grid
- Branch: claude/teletype
- Changed: src/apps/norns/host.rs (GridHost: `_px.grid_*` functions, key and
  resize delivery each tick, rotation both ways, dark LEDs on stop),
  src/apps/norns/lua/prelude.lua (real `grid.connect`, vports 1-4, led/all/
  refresh/rotation/intensity, `g.key`, `grid.add`/`grid.remove` on resize),
  src/apps/norns/mod.rs (passes the grid to the script thread), new bundled
  script assets/norns/code/gridsteps (a step sequencer sized to the grid).
- Status: done. A script that calls `grid.connect()` takes grid focus as
  client "Norns"; port 1 is the shared grid (on-screen or hardware), 2-4 stay
  unattached. `intensity`/`tilt_enable` are accepted and ignored.
- Tests: cargo test --bin portamax-sim -- norns.

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
  The arc, Teletype's G.* ops and the norns grid API came after (entries above).
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
