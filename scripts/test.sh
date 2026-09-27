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
# Runtime is a separate package, so the root package's tests do not include it.
# Explicit arguments retain the caller's focused target selection.
if [ "$#" -eq 0 ]; then
    cargo test --manifest-path crates/runtime/Cargo.toml
fi
cargo test "$@"
