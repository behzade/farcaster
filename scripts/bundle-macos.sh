#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
bundle=${FARCASTER_BUNDLE_PATH:-"$target_dir/release/Farcaster.app"}

case $bundle in
    *.app) ;;
    *)
        echo "FARCASTER_BUNDLE_PATH must end in .app: $bundle" >&2
        exit 1
        ;;
esac

cargo build --manifest-path "$root/Cargo.toml" --release

rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$target_dir/release/farcaster" "$bundle/Contents/MacOS/farcaster"
cp "$root/packaging/macos/Info.plist" "$bundle/Contents/Info.plist"
cp "$root/NOTICE.md" "$bundle/Contents/Resources/NOTICE.md"

identity=${CODESIGN_IDENTITY:--}
sign() {
    if [ "$identity" = "-" ]; then
        codesign --force --sign - "$1"
    else
        codesign --force --options runtime --timestamp --sign "$identity" "$1"
    fi
}
sign "$bundle/Contents/MacOS/farcaster"
sign "$bundle"
codesign --verify --deep --strict "$bundle"

printf '%s\n' "$bundle"
