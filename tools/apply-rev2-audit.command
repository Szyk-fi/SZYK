#!/bin/bash
# Apply this reviewed branch to Max's normal checkout, then optionally run Slint.
set -euo pipefail
audit_dir="$(cd "$(dirname "$0")/.." && pwd)"
checkout="/Users/max/Developer/Modulade/portamax-sim"
branch="gpt/rev2-parameter-ui-audit"
base="977d8c37dab2e8dd874943fe73e60c6a85c86dbc"
if [ "$(git -C "$checkout" branch --show-current)" != main ]; then
    echo "Stop: the Portamax checkout must be on main before applying this branch." >&2
    exit 1
fi
if ! git -C "$checkout" diff --quiet --ignore-submodules=dirty || ! git -C "$checkout" diff --cached --quiet; then
    echo "Stop: tracked local changes must be saved before applying. Nothing was changed." >&2
    exit 1
fi
# Check the current remote before changing source files. If main advanced after
# this audit, require a rebase and verification instead of guessing a merge.
git -C "$checkout" fetch origin main
if [ "$(git -C "$checkout" rev-parse FETCH_HEAD)" != "$base" ] || [ "$(git -C "$checkout" rev-parse HEAD)" != "$base" ]; then
    echo "Stop: main advanced after this audit. Rebase and retest the audit branch first." >&2
    exit 1
fi
git -C "$checkout" fetch "$audit_dir" "$branch"
git -C "$checkout" merge --ff-only FETCH_HEAD
echo "Rev2 audit applied. Imported patches and local submodule source were preserved."
if [ "${1:-}" = --run ]; then
    cd "$checkout"
    exec cargo run --offline --example slint_home_live
fi
echo "Run the Slint UI with: cd '$checkout' && cargo run --offline --example slint_home_live"
