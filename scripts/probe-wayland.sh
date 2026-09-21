#!/usr/bin/env bash
# Shared compositor lifecycle for the package startup probes.
stop_probe_wayland() {
    if [ -n "$weston_pid" ]; then
        kill "$weston_pid" 2>/dev/null || true
        wait "$weston_pid" 2>/dev/null || true
    fi
    kill "$xvfb_pid" 2>/dev/null || true
    wait "$xvfb_pid" 2>/dev/null || true
}

start_probe_wayland() {
    local logs=$1
    # GPUI needs wl_seat, which Weston's headless backend does not provide.
    Xvfb :99 -screen 0 1280x800x24 -nolisten tcp -ac > "$logs/xvfb.log" 2>&1 &
    xvfb_pid=$!
    weston_pid=
    trap stop_probe_wayland EXIT
    for ((attempt=0; attempt<100; attempt++)); do
        [ ! -S /tmp/.X11-unix/X99 ] || break
        kill -0 "$xvfb_pid"
        sleep 0.1
    done
    test -S /tmp/.X11-unix/X99
    DISPLAY=:99 weston --backend=x11-backend.so --use-pixman --no-config \
        --socket="$WAYLAND_DISPLAY" --idle-time=0 --width=1280 --height=800 \
        > "$logs/weston.log" 2>&1 &
    weston_pid=$!
    for ((attempt=0; attempt<100; attempt++)); do
        if [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]; then
            break
        fi
        if ! kill -0 "$weston_pid"; then
            cat "$logs/weston.log"
            return 1
        fi
        sleep 0.1
    done
    test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
    timeout 10s wayland-info > "$logs/wayland-info.txt" 2>&1
    grep -q wl_seat "$logs/wayland-info.txt"
}
