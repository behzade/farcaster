#!/bin/zsh -f
set -euo pipefail
unsetopt BG_NICE

binary=${1:?usage: run-macos.sh executable [args...]}
shift

# Cargo puts test and benchmark executables beside thousands of object files.
# macOS scans that directory while looking for an app bundle, which can occupy
# syspolicyd's workers and stall unrelated program launches. Keep normal
# `cargo run` paths intact; stage only executables from a deps directory.
case "$binary" in
    deps/*|*/deps/*) ;;
    *) exec "$binary" "$@" ;;
esac

deps_dir=$(cd "$(dirname "$binary")" && pwd -P)
run_dir=$(mktemp -d "$deps_dir/../.farcaster-test-run.XXXXXXXX")
staged="$run_dir/$(basename "$binary")"
child=

cleanup() {
    /bin/rm -f "$staged"
    /bin/rmdir "$run_dir"
}
trap cleanup EXIT

# APFS clones avoid copying the large executable's data. Fall back for other
# filesystems. Leave Cargo's library paths, working directory and args intact.
/bin/cp -c "$binary" "$staged" 2>/dev/null || /bin/cp "$binary" "$staged"

forward_signal() {
    if [[ -n "$child" ]]; then
        kill -s "$1" "$child" 2>/dev/null || true
    fi
}
trap 'forward_signal INT' INT
trap 'forward_signal TERM' TERM
trap 'forward_signal HUP' HUP

# zsh lets the background child restore SIGINT before exec.
(
    trap - INT TERM HUP
    exec "$staged" "$@"
) <&0 &
child=$!

exit_status=0
wait "$child" || exit_status=$?
# A trapped signal interrupts wait before the child has necessarily exited.
# Reap it before removing the executable, including tests that re-exec it.
while kill -0 "$child" 2>/dev/null; do
    wait "$child" || exit_status=$?
done
exit "$exit_status"
