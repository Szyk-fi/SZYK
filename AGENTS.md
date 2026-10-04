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
