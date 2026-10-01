# Oracle

Describe a sound in plain words and Oracle builds it: a synth, a drum voice, an effect or a self-playing generator, ready to play on the pads.

Behind the scenes, an AI writes a **patch**: a small JSON description of an audio graph made from Oracle's 21 building blocks, plus small math formulas. Oracle checks the patch and compiles it. If the patch is broken, Oracle sends the errors back to the AI to fix, up to twice. Nothing the AI writes can crash the device or glitch the audio.

## Asking for a sound

**Type it.** The terminal running `cargo run` is Oracle's prompt box. Type a description and press Enter:

```
a glassy FM pluck that blooms into a shimmer reverb when you hold the pad
```

| Type | Does |
|------|------|
| `<description>` | generate a new patch |
| `+ darker, more percussive` | refine the current patch (also `/refine ...`) |
| `/vary` | AI variation of the current patch |
| `/explain` | what the patch does and what to try |
| `/breed <library name>` | AI-breed the current patch with another one |
| `/random 0.5` | randomize unlocked params |
| `/undo` `/redo` | walk the history |
| `/save [name]` `/load <name>` `/list` | patch library |
| `/json` | print the current patch |
| paste JSON | load a patch someone shared |

**On the device, no typing.** Open **Ask the Oracle**, turn the Kind / Mood / Texture / Motion word wheels with knob 2, then press knob 2 on **Generate**. **Refine** works the same way, with a wheel of instructions.

**Say it.** In **Ask the Oracle**, press knob 2 on **Speak a new patch** and describe the sound out loud. Press knob 2 again when you're done, or just stop talking; a short pause ends the take, and takes are capped at 15 seconds. **Speak a change** works the same way for edits, like "make it darker and slower". The words are turned into text and sent to the Oracle exactly as if you had typed them. The terminal shows what it heard.

The voice comes from the computer's default input in the sim (on a Mac, allow microphone access for your terminal app the first time). Speech-to-text needs one of:

| Start the sim with | Uses |
|---------------------|------|
| `ORACLE_STT_URL=http://127.0.0.1:8080/v1 cargo run` | a local Whisper server (whisper.cpp, faster-whisper-server, LocalAI): works fully offline |
| `OPENAI_API_KEY=... cargo run` | OpenAI's hosted transcription |
| nothing | the take is saved to `apps/oracle/voice/last.wav`, nothing is transcribed |

Optional: `ORACLE_STT_KEY` (bearer key for your server), `ORACLE_STT_MODEL` (default `whisper-1`), `ORACLE_STT_LANG` (for example `en`). `ORACLE_STT_URL` also accepts a full endpoint ending in `/inference` (whisper.cpp's own server) or `/audio/transcriptions`.

Speech-to-text and the patch AI are set up separately, so you can mix them, for example a local Whisper server for your voice and Claude for the patches.

## AI setup

| Start the sim with | Uses |
|---------------------|------|
| `ANTHROPIC_API_KEY=sk-ant-... cargo run` | Claude (`ORACLE_MODEL` to pick the model) |
| `ORACLE_LLM_URL=http://localhost:11434/v1 ORACLE_MODEL=qwen2.5-coder cargo run` | any OpenAI-compatible server (Ollama, LM Studio) |
| `OPENAI_API_KEY=... ORACLE_MODEL=... cargo run` | OpenAI |
| nothing | manual mode: Oracle writes `apps/oracle/prompt.txt`; paste it into any chatbot, then paste the JSON reply into the terminal |


**Keep keys in `.env`.** Instead of typing them on the command line, put them in a file named `.env` in the project folder, one per line, and just `cargo run`:

```
ANTHROPIC_API_KEY=sk-ant-...
ORACLE_STT_URL=http://127.0.0.1:8080/v1
```

`.env` is gitignored, so keys there are never committed or pushed. A variable set in your shell overrides the file.

## Playing

- **Pages 1-4**: up to 64 named parameters. Knob 2 edits, knob 2 press resets, knob 1 press **locks** a param so randomize and mutate leave it alone.
- **Macros**: 8 performance knobs the AI designed, each moving several params.
- **Play**: level, voices, glide, root and scale for the pads, transpose, Hold (pads latch, and drones keep playing after you leave the app), amp ADSR, tempo, Source (another app's audio as input) and dry/wet.
- **F3**: play/stop, for generator patches (the ones that play on their own). Instruments and effects have no transport; their clocks, sequencers and tempo delays always run.
- **Pads = Snap** (Play > Pads): tap an empty pad to store a snapshot, tap a stored pad to recall it, hold 0.6 s to overwrite, and hold two pads to set up a **morph**. Then turn Morph & Evolve > Morph.
- **Morph & Evolve**: Randomize (with an amount), **Mutate x4** puts 4 variations on the bottom-right pads, **Breed A x B** makes 4 children of the two morph snapshots, and **AI breed with** combines the current patch with a library patch.
- **Library**: 10 starters, your saved patches (`apps/oracle/patches/*.json`), save, undo and redo.

Other apps can modulate Oracle through ModBus: `Oracle: Macro 1-8`, `Oracle: Morph`, and `Oracle: P1-P16` (the first 16 params of whatever patch is loaded).

## Writing patches by hand

Patches are plain JSON. See `src/apps/oracle/starters/` for 10 examples, and run `/json` to see the current one. The full language reference is the AI's system prompt, generated from the code in `src/apps/oracle/llm.rs` (`system_prompt()`).
