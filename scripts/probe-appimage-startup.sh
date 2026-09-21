#!/usr/bin/env bash
# Exercise the packaged binary; no mocked graphics calls.
set -euo pipefail

appimage=$(realpath "${1:?usage: probe-appimage-startup.sh APPIMAGE LOG_DIR}")
mkdir -p "${2:?usage: probe-appimage-startup.sh APPIMAGE LOG_DIR}"
logs=$(realpath "$2")
appimage_runner=()
if [ -n "${FARCASTER_PROBE_APPIMAGE_RUNNER:-}" ]; then
    appimage_runner+=("$FARCASTER_PROBE_APPIMAGE_RUNNER")
fi
probe_dir=$(mktemp -d)
export XDG_RUNTIME_DIR="$probe_dir/runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"
export WAYLAND_DISPLAY=farcaster-probe
export LIBGL_ALWAYS_SOFTWARE=1
unset DISPLAY LD_PRELOAD LD_LIBRARY_PATH
ulimit -c 0

{
    uname -a
    cat /etc/os-release
    weston --version
    file "$appimage"
    if command -v pacman >/dev/null; then
        pacman -Q
    elif command -v dpkg-query >/dev/null; then
        dpkg-query -W
    else
        nixos-version
        readlink -f /run/current-system
    fi
} > "$logs/environment.txt"

host_wayland=${FARCASTER_PROBE_HOST_WAYLAND:-}
if [ -z "$host_wayland" ]; then
    host_wayland=$(ldconfig -p | awk '/libwayland-client.so.0 / { if (!found) { print $NF; found=1 } }')
fi
test -f "$host_wayland"
printf '%s\n' "$host_wayland" > "$logs/host-wayland.txt"
readelf --dyn-syms --wide "$host_wayland" > "$logs/host-wayland-symbols.txt"

# shellcheck source=scripts/probe-wayland.sh
source "$(dirname "$0")/probe-wayland.sh"
start_probe_wayland "$logs"

printf '| Attempt | Exit code | Result |\n| --- | --- | --- |\n' > "$logs/summary.md"
failed=0
for mode in baseline host-preload; do
    run_dir="$probe_dir/$mode"
    mkdir -p "$run_dir/project" "$run_dir/config" "$run_dir/data" "$logs/$mode"
    extra_env=()
    if [ "$mode" = host-preload ]; then
        extra_env+=("LD_PRELOAD=$host_wayland")
    fi
    # Extract-and-run preserves AppRun's library setup and works without FUSE.
    # LD_DEBUG writes per-process files, including dlopen and symbol failures.
    if timeout --signal=TERM --kill-after=3s 20s env \
        APPIMAGE_EXTRACT_AND_RUN=1 RUST_BACKTRACE=full WAYLAND_DEBUG=client \
        LD_DEBUG=libs,versions LD_DEBUG_OUTPUT="$logs/$mode/loader" \
        XDG_DATA_HOME="$run_dir/data" XDG_CONFIG_HOME="$run_dir/config" \
        FARCASTER_DATA_DIR="$run_dir/data/farcaster" \
        "${extra_env[@]}" "${appimage_runner[@]}" "$appimage" "$run_dir/project" \
        > "$logs/$mode/startup.log" 2>&1; then
        status=0
    else
        status=$?
    fi
    printf '%s\n' "$status" > "$logs/$mode/exit-code.txt"
    startup="$logs/$mode/startup.log"
    if grep -Eq 'khronos[-_]egl' "$startup" && grep -q 'panicked' "$startup"; then
        result='EGL panic reproduced'
        failed=1
    elif [ "$status" -eq 124 ] \
        && grep -q 'Selected GPU adapter:' "$startup" \
        && grep -Eq 'wl_surface[@#][0-9]+\.attach\(' "$startup" \
        && grep -Eq 'wl_surface[@#][0-9]+\.commit\(' "$startup"; then
        result='Window rendered; stopped at timeout'
    else
        result='Startup failed or rendering unconfirmed; inspect logs'
        failed=1
    fi
    printf '| %s | %s | %s |\n' "$mode" "$status" "$result" >> "$logs/summary.md"
    tail -25 "$startup"
done

# Record the actual bundle's libraries after testing, without changing it.
mkdir "$probe_dir/extracted"
(
    cd "$probe_dir/extracted"
    if [ "${#appimage_runner[@]}" -gt 0 ]; then
        "${appimage_runner[@]}" -x "$PWD/squashfs-root" "$appimage" > "$logs/extraction.log" 2>&1
    else
        "$appimage" --appimage-extract > "$logs/extraction.log" 2>&1
    fi
    find squashfs-root -type f -name '*.so*' -print > "$logs/bundled-libraries.txt"
    while IFS= read -r library; do
        printf '\n%s\n' "$library"
        readelf --dyn-syms --wide "$library"
    done < <(find squashfs-root -name 'libwayland-client.so*' -type f)
) > "$logs/bundled-wayland-symbols.txt" 2>&1
printf '\nSoftware rendering only; this does not cover hardware GPU drivers.\n' >> "$logs/summary.md"
cat "$logs/summary.md"
exit "$failed"
