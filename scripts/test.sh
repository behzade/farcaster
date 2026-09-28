#!/bin/sh
# Isolate Farcaster data for this invocation while preserving Cargo caches,
# toolchain settings, and the caller's HOME. App/E2E fixtures keep their own
# narrower isolation as well.
set -eu

run_data_dir=$(mktemp -d "${TMPDIR:-/tmp}/farcaster-tests.XXXXXXXX")
trap 'rm -rf "$run_data_dir"' 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

export FARCASTER_DATA_DIR="$run_data_dir"
# Explicit arguments retain the caller's focused target selection.
if [ "$#" -gt 0 ]; then
    cargo test "$@"
    exit
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$repo_root"
# Each package owns its unit tests; testing a dependency's caller only builds it.
# Use a separate descriptor so Cargo and tests keep the caller's standard input.
while IFS= read -r manifest <&3 || [ -n "$manifest" ]; do
    case "$manifest" in
        ''|'#'*) continue ;;
    esac
    cargo test --manifest-path "$manifest"
done 3< "$repo_root/scripts/first-party-packages.txt"
