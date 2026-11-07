#!/bin/sh
set -eu

[ "$(uname -s)" = Darwin ] || { echo 'This launcher requires macOS.' >&2; exit 1; }
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
pointer=/private/tmp/farcaster-ui-instance-path
bundle=$(cat "$pointer" 2>/dev/null || true)
if [ ! -d "$bundle" ]; then
    directory=$(mktemp -d /private/tmp/farcaster-ui-XXXXXX)
    bundle="$directory/Farcaster Isolated.app"
    cp -R "$target_dir/release/Farcaster.app" "$bundle"
    printf '%s\n' "$bundle" > "$pointer"
fi

# Reuse the existing process without replacing its executable or creating another.
if ps -axo comm= | grep -Fxq -- "$bundle/Contents/MacOS/farcaster"; then
    open -g "$bundle"
    printf '%s\n' "$bundle"
    exit 0
fi
cp "$target_dir/release/farcaster" "$bundle/Contents/MacOS/farcaster"
cp "$root/assets/icons/app/FarcasterIsolated.icns" "$bundle/Contents/Resources/"
plist="$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier io.github.behzade.farcaster.isolated' "$plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleName Farcaster Isolated' "$plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName Farcaster Isolated' "$plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIconFile FarcasterIsolated.icns' "$plist"
codesign --force --sign - --identifier io.github.behzade.farcaster.isolated \
    --requirements '=designated => identifier "io.github.behzade.farcaster.isolated"' "$bundle"

# Launch Services reuses this bundle; -g leaves the user's current app active.
if [ -n "${FARCASTER_DATA_DIR:-}" ]; then
    open -g --env "FARCASTER_DATA_DIR=$FARCASTER_DATA_DIR" "$bundle" --args --isolated "${1:-$root}"
else
    open -g "$bundle" --args --isolated "${1:-$root}"
fi
printf '%s\n' "$bundle"
