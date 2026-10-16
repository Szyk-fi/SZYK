#!/bin/bash
# One-time source integration. ROVE itself starts Push support on normal launch.
set -euo pipefail
review="$(cd "$(dirname "$0")/.." && pwd)"
checkout="${1:-/Users/max/Developer/Modulade/portamax-sim}"
base="c52077a00631bc266fe4f857c2caa9efd855fb5e"
if [ "$#" -gt 1 ]; then
    echo "Usage: bash $0 [existing-checkout]" >&2
    exit 2
fi
checkout="$(git -C "$checkout" rev-parse --show-toplevel)"
if [ "$review" = "$checkout" ]; then
    echo "This checkout already contains native Push 2 support. Launch ROVE normally."
    exit 0
fi
git -C "$review" merge-base --is-ancestor "$base" HEAD
patch_file="$(mktemp "${TMPDIR:-/tmp}/rove-push2.XXXXXX")"
trap 'rm -f "$patch_file"' EXIT
# Recognize an installed revision before preparing its incremental update. This
# also works when earlier changes are still uncommitted in the target checkout.
for previous in $(git -C "$review" rev-list --first-parent "$base..HEAD") "$base"; do
    if [ "$previous" != "$base" ]; then
        git -C "$review" diff --binary "$base" "$previous" > "$patch_file"
        if ! git -C "$checkout" apply --reverse --check "$patch_file" >/dev/null 2>&1; then
            continue
        fi
    fi
    git -C "$review" diff --binary "$previous" HEAD > "$patch_file"
    if [ ! -s "$patch_file" ]; then
        echo "Native Push 2 defaults are already current. Launch ROVE normally."
        exit 0
    fi
    # Check every hunk before writing. Keep branch, index and existing local edits.
    git -C "$checkout" apply --check "$patch_file"
    git -C "$checkout" apply "$patch_file"
    echo "Native Push 2 defaults updated. Launch ROVE normally."
    exit 0
done
