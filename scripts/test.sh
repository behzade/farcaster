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
cargo test "$@"
