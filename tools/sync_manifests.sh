#!/bin/sh
# Rewrites the `mod_inputs` list in every apps/*/manifest.toml from what the
# app's code registers on the ModBus (registry.rs checks the two agree). Run
# this after adding or renaming a modulation input, then commit the manifests.
cd "$(dirname "$0")/.." || exit 1
PORTAMAX_WRITE_MANIFESTS=1 cargo test --bin portamax-sim every_manifest_declares_exactly_the_inputs_its_app_registers
