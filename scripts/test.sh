#!/bin/sh
set -eu

run_data_dir=$(mktemp -d "${TMPDIR:-/tmp}/farcaster-tests.XXXXXXXX")
trap 'rm -rf "$run_data_dir"' 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

export FARCASTER_DATA_DIR="$run_data_dir"
if [ "$#" -gt 0 ]; then
    cargo test --locked "$@"
    exit
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
cd "$repo_root"
test_status=0
while IFS= read -r manifest <&3 || [ -n "$manifest" ]; do
    case "$manifest" in
        ''|'#'*) continue ;;
    esac
    if cargo test --locked --manifest-path "$manifest"; then
        :
    else
        package_status=$?
        if [ "$test_status" -eq 0 ]; then
            test_status=$package_status
        fi
        printf 'Tests failed for %s (exit %s)\n' "$manifest" "$package_status" >&2
    fi
done 3< "$repo_root/scripts/first-party-packages.txt"
exit "$test_status"
