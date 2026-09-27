# Portamax independent reliability review

## Completed

- **Cascade phrase gain:** reproduced a reset from two-voice headroom to one during a silent operator/modulation interval, despite notes still being held. Headroom now follows held/releasing voice lifetimes and resets only after the phrase finishes. Regression covers silent intervals, releasing the second note, and the final return to idle. This fixes a concrete gain-jump mechanism; it does not prove the exact originally reported incident is fully resolved.
- **Scope resource use:** a configured input no longer forces a closed Scope to run indefinitely. Normal visibility/grace and explicit output-consumer demand still wake it. Its source choice survives sleep.
- **Monitoring lifecycle:** Analyzer and Visualizer explicitly keep running with Monitor On; Monitor Off lets them sleep unless another app needs their output.
- **Master gain isolation:** non-finite gain values yield silence rather than non-finite audio, and a subsequent valid gain restores output.
- **Live example build:** added the missing NeoGeo module and re-export to slint_home_live. The module was present in the main application but absent from the example's mirrored module tree.

Current visual designs and preset definitions were preserved. No manufacturing, publishing, deployment, or hardware settings were changed.

## Validation

- `cargo test --offline --bin portamax-sim`: **422 passed, 0 failed, 2 ignored**. The ignored tests are optional asset-generating tests. NeoGeo tests passed on this checkout; its engine was not edited in this pass.
- Opt-in Cascade listening export: passed; four six-second 48 kHz / 24-bit mono WAVs plus measurements.
- `cargo run --offline --example slint_home_live -- --render-instruments …`: **53 isolated-app checks and 53 main-screen renders** passed. Includes finite output smoke checks, available transport actions, source catalog stability, D-pad timing, joystick, shoulders, F1–F4, and all sixteen pads. This is software rendering and simulated input, not a physical MIDI/hardware test.
- Routing lifecycle integration: a source is constructed only when requested, publishes readable audio, sleeps after the final consumer closes, clears stale output, and resumes without reconstruction.
- Existing 100-app regression: zero app construction or DSP calls for dormant installed slots.
- `git diff --check`: passed.

## Listening review

Open index.html. Each clip starts with one note, adds another at 1.49 s, releases the added note at 2.51 s, then releases all notes at 4 s. The remaining two seconds capture release. The Cascade mixer remains at 0.60 throughout. Clips contain raw module output, before the global mixer limiter; there is no loudness normalization.

| Patch | Variant | Peak | RMS |
|---|---|---:|---:|
| SteelCans | Imported preset | 0.519490 | 0.085764 |
| SteelCans | OP5 muted | 0.519477 | 0.085764 |
| W. BLOCK 1 | Imported preset | 0.581111 | 0.074180 |
| W. BLOCK 1 | OP5 muted | 0.397729 | 0.050204 |

Both presets remained finite and within the fixed 60% module gain ceiling during the automated sequence. OP5 makes little measured difference to SteelCans in this case and a substantial difference to W. BLOCK 1. Muted variants are diagnostic comparisons; saved presets remain unchanged. Bounded audio does not establish perceptual quality, DX7 fidelity, or freedom from aliasing.

## Notes to bring back

1. Whether Cascade still jumps in perceived volume during your normal playing.
2. Which W. BLOCK 1 version sounds preferable, and whether its roughness sounds intentional.
3. Any routing or app-switching behavior that interrupts your workflow.

## Remaining validation limits

Physical controller/MIDI behavior, device switching, speaker/headphone playback, target-hardware CPU/RAM budgets, and hardware latency need real-device validation. Dormant-app and route-demand checks establish software behavior, not a measured embedded power or memory budget. This pass did not exhaustively audit every DSP parameter or establish that all app engines are sonically equivalent to their inspirations.

## Files touched in this pass

- src/apps/cascade.rs: voice-lifecycle normalization, regression and listening export.
- src/apps/collection.rs: Scope background-activity rule and regression.
- src/apps/analyzer.rs, src/apps/visualizer.rs: monitoring activity rules and tests.
- src/audio.rs: invalid master gain protection and test.
- src/app_runtime.rs: routed source wake/suspend/resume integration test.
- examples/slint_home_live.rs: NeoGeo module wiring.

Logs are in logs/. Audio and raw measurements are in audio/. Screens are in screens/.
