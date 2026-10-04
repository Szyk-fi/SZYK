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

### 2026-10-04: Claude: control contract DRAFT (docs only)
- Branch: claude/atlas-play   Commits: see git log on the branch
- Changed: new `docs/CONTROL_CONTRACT.md` only. **No code changed.**
- Status: draft, waiting on Max's decisions (section 8 of the doc).
- Tests: none needed (docs).
- Notes for the other assistant: please don't build against it until Max
  approves. If approved it will change what D-pad, SELECT, L1, R1, F2 and
  the stick mean, and add a contract test every app must pass. That
  includes the gamepad layout in `controller_map.rs` (your area): the PS5
  navigate map would change so the D-pad means the same everywhere.
- Request: none yet. If approved, a request will cover the
  `controller_map.rs` defaults.

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
