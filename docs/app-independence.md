# App independence and controls

App independence is a system requirement. An app must not require another app to be installed merely to construct, display, accept controls, or process an unpatched audio block.

## Current supported boundary

The installed inventory is discovered from `apps/*/manifest.toml` at startup. Code is compiled into the executable and selected by stable manifest ID in `Registry`. Adding an implemented app's manifest or removing it changes the next launch's inventory. Display names are presentation, not service identity. Missing/unknown implementations are skipped; duplicate IDs are instantiated once.

Mixer and Settings expose `SystemRole`; launcher shortcuts query that role and are disabled when the service is absent. Apps declare pad and transport capabilities through `App`. The shell must not infer behavior from display names or app-list positions.

F1 opens Home (Settings from Home when installed). F2 is contextual: Sequencer pad/step mode, otherwise Pad Lock/Unlock for supported instruments. Pad Lock retains the instrument's pad input while another screen is open. F3 displays the app's next transport action. F4 opens Mixer when installed. Prism's utility exposes Record / Play / Clear or Hold / Release. The legacy framebuffer runtime uses F2 for its existing per-app MIDI input arm instead of the Slint pad lock; its label reflects that difference. F3 is transport in both runtimes.

## Dependency rules

- Share audio and modulation through injected buses; do not look up another app by display name or assume it occupies a specific slot.
- Missing audio input means unpatched/silent input, not index zero. Cycling an empty source inventory stays unpatched.
- Each app owns its state and processor. Construction must not mutate unrelated app state. Audio processors remain registered while navigating; leaving a screen is not uninstalling its app.
- Release held pad input when its owner changes. Transport remains independent of screen navigation.
- Optional system services may be absent. Their shortcuts must become unavailable.
- Hardware discovery/output must be injectable for offline tests.

## Verification

`cargo test --offline --bin portamax-sim` exercises 322 tests, including duplicate/missing/renamed entries, no-source routing, Pam's clock pause/resume and Prism utility actions.

`cargo run --offline --example slint_home_live -- --render-instruments OUTPUT_DIRECTORY` constructs each of 31 implemented apps with fresh shared buses and no peers, checks its menu/visual payload, processes finite audio where applicable, and verifies transport toggling. It then renders the collection and checks pointer navigation, disabled F-buttons, all pads, joystick and D-pad repeat timing. This is a startup smoke test, not exhaustive DSP interoperability or physical-device verification.

## Remaining architecture work

This is not a hot-unload plugin system. Audio/modulation routes currently use session-local indices, and processors/bus registrations live for the session. Do not persist those indices as portable patches. Live removal requires stable app-instance/port IDs, registration ownership and unregister operations, safe audio-thread graph swaps, held-note cleanup, and unresolved-route handling. Saved patches must retain unresolved stable IDs rather than silently reconnecting by index.

Compiled app code and its Slint visual variants also remain centrally registered. Removing a manifest needs no peer changes; deleting implementation source still requires updating that compiled registration. A fully independently packaged application format is not implemented.

The twenty new concepts are a separate `slint_new_apps` UI study executable. They do not register unfinished audio/storage/network implementations in the working inventory. Each concept has its own control and pad state; the study's F-buttons browse/control previews, not audio transport. Integrate future working apps through the same capability contract, with explicit audio, recording, storage and resource-lifecycle tests.
