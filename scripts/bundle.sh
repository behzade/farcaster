#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
project=${PROJECT:-"$root"}
action=${1:-bundle}

case $action in
    bundle|--relaunch) ;;
    *)
        echo "usage: $0 [--relaunch]" >&2
        exit 2
        ;;
esac

platform=$(uname -s)
case $platform in
    Darwin)
        formats=app
        ;;
    Linux)
        formats=${BUNDLE_FORMATS:-appimage}
        ;;
    *)
        echo "Farcaster bundles support macOS and Linux" >&2
        exit 1
        ;;
esac

if [ "$platform" = "Linux" ] && [ "$action" = "--relaunch" ]; then
    case $formats in
        appimage|appimage,*|*,appimage|*,appimage,*) ;;
        *)
            echo "bundle-relaunch requires appimage in BUNDLE_FORMATS" >&2
            exit 2
            ;;
    esac
fi

mkdir -p "$target_dir/release"
CARGO_TARGET_DIR="$target_dir" cargo packager --release --formats "$formats" \
    --out-dir "$target_dir/release"

if [ "$platform" = "Darwin" ]; then
    bundle="$target_dir/release/Farcaster.app"
    if [ ! -d "$bundle" ]; then
        echo "macOS bundle not found: $bundle" >&2
        exit 1
    fi
    identity=${CODESIGN_IDENTITY:--}
    if [ "$identity" = "-" ]; then
        codesign --force --sign - "$bundle/Contents/MacOS/farcaster"
        codesign --force --sign - "$bundle"
    else
        codesign --force --options runtime --timestamp --sign "$identity" \
            "$bundle/Contents/MacOS/farcaster"
        codesign --force --options runtime --timestamp --sign "$identity" "$bundle"
    fi
    codesign --verify --deep --strict "$bundle"
fi

if [ "$action" != "--relaunch" ]; then
    exit 0
fi

wait_for_linux_exit() {
    attempts=0
    while pgrep -x farcaster >/dev/null 2>&1; do
        if [ "$attempts" -ge 50 ]; then
            echo "Farcaster did not stop within five seconds" >&2
            exit 1
        fi
        attempts=$((attempts + 1))
        sleep 0.1
    done
}

case $platform in
    Darwin)
        osascript -e 'if application id "io.github.behzade.farcaster" is running then tell application id "io.github.behzade.farcaster" to quit' \
            >/dev/null 2>&1 || true
        attempts=0
        while osascript -e 'application id "io.github.behzade.farcaster" is running' \
            2>/dev/null | grep -q true; do
            if [ "$attempts" -ge 50 ]; then
                echo "Farcaster did not stop within five seconds" >&2
                exit 1
            fi
            attempts=$((attempts + 1))
            sleep 0.1
        done
        open -n "$bundle" --args "$project"
        ;;
    Linux)
        appimage=$(find "$target_dir/release" -maxdepth 1 -type f -iname '*farcaster*.AppImage' \
            -printf '%T@ %p\n' | sort -nr | sed -n '1s/^[^ ]* //p')
        if [ -z "$appimage" ]; then
            echo "Farcaster AppImage not found in $target_dir/release" >&2
            exit 1
        fi
        pkill -TERM -x farcaster >/dev/null 2>&1 || true
        wait_for_linux_exit
        nohup "$appimage" "$project" >/dev/null 2>&1 &
        ;;
esac
