#!/usr/bin/env bash
# Launch an installed native package under a real Wayland compositor.
set -euo pipefail
binary=$(realpath "${1:?usage: probe-native-startup.sh INSTALLED_BINARY LOG_DIR}")
mkdir -p "${2:?usage: probe-native-startup.sh INSTALLED_BINARY LOG_DIR}"
logs=$(realpath "$2")
probe_dir=$(mktemp -d)
export XDG_RUNTIME_DIR="$probe_dir/runtime"
export XDG_CONFIG_HOME="$probe_dir/config"
export XDG_DATA_HOME="$probe_dir/data"
export FARCASTER_DATA_DIR="$probe_dir/data/farcaster"
mkdir -m700 "$XDG_RUNTIME_DIR"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$probe_dir/project"
export WAYLAND_DISPLAY=farcaster-probe
export LIBGL_ALWAYS_SOFTWARE=1
unset DISPLAY LD_PRELOAD LD_LIBRARY_PATH
ulimit -c 0
{
    uname -a
    cat /etc/os-release
    file "$binary"
    weston --version
} > "$logs/environment.txt"

# shellcheck source=scripts/probe-wayland.sh
source "$(dirname "$0")/probe-wayland.sh"
start_probe_wayland "$logs"

status=0
timeout --signal=TERM --kill-after=3s 20s env \
    RUST_BACKTRACE=full WAYLAND_DEBUG=client \
    LD_DEBUG=libs,versions LD_DEBUG_OUTPUT="$logs/loader" \
    "$binary" "$probe_dir/project" > "$logs/startup.log" 2>&1 || status=$?
printf '%s\n' "$status" > "$logs/exit-code.txt"
tail -30 "$logs/startup.log"
if [ "$status" -ne 124 ] \
    || grep -q 'panicked' "$logs/startup.log" \
    || ! grep -q 'Selected GPU adapter:' "$logs/startup.log" \
    || ! grep -Eq 'wl_surface[@#][0-9]+\.attach\(' "$logs/startup.log" \
    || ! grep -Eq 'wl_surface[@#][0-9]+\.commit\(' "$logs/startup.log"; then
    printf 'Native package startup failed or rendering unconfirmed (exit %s).\n' "$status" > "$logs/summary.md"
    exit 1
fi
printf 'Installed native package rendered a window and ran until timeout.\n\nSoftware rendering only; hardware GPU drivers and terminal/editor interactions remain untested.\n' > "$logs/summary.md"
