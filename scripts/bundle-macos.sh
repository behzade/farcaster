#!/bin/sh
set -eu

NONO_VERSION=0.61.1
NONO_AARCH64_SHA256=922d0d4f86720b34d1d1e8a0f141c7a883d98ac20749869545ecd3412a07a1f9
NONO_X86_64_SHA256=b9ba6e92b33b7a543a9acfc2f3ba431bec75a30d1a613024dc9ca79094f72af7

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
bundle=${FARCASTER_BUNDLE_PATH:-"$target_dir/release/Farcaster.app"}

case $(uname -m) in
    arm64|aarch64)
        target=aarch64-apple-darwin
        expected_sha256=$NONO_AARCH64_SHA256
        ;;
    x86_64)
        target=x86_64-apple-darwin
        expected_sha256=$NONO_X86_64_SHA256
        ;;
    *)
        echo "unsupported macOS architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

case $bundle in
    *.app) ;;
    *)
        echo "FARCASTER_BUNDLE_PATH must end in .app: $bundle" >&2
        exit 1
        ;;
esac

cargo build --manifest-path "$root/Cargo.toml" --release

temporary=$(mktemp -d "${TMPDIR:-/tmp}/farcaster-bundle.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
archive="$target_dir/release/nono-v$NONO_VERSION-$target.tar.gz"
url="https://github.com/nolabs-ai/nono/releases/download/v$NONO_VERSION/nono-v$NONO_VERSION-$target.tar.gz"

archive_is_valid() {
    [ -f "$1" ] && [ "$(shasum -a 256 "$1" | awk '{print $1}')" = "$expected_sha256" ]
}

if ! archive_is_valid "$archive"; then
    download="$temporary/nono.tar.gz"
    curl --fail --location --silent --show-error --output "$download" "$url"
    if ! archive_is_valid "$download"; then
        actual_sha256=$(shasum -a 256 "$download" | awk '{print $1}')
        echo "nono archive checksum mismatch: expected $expected_sha256, got $actual_sha256" >&2
        exit 1
    fi
    mv "$download" "$archive"
fi

tar -xzf "$archive" -C "$temporary" nono
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/licenses"
cp "$target_dir/release/farcaster" "$bundle/Contents/MacOS/farcaster"
cp "$temporary/nono" "$bundle/Contents/MacOS/nono"
cp "$root/packaging/macos/Info.plist" "$bundle/Contents/Info.plist"
cp "$root/NOTICE.md" "$bundle/Contents/Resources/NOTICE.md"
cp "$root/THIRD_PARTY_LICENSES/NONO-APACHE-2.0.txt" \
    "$bundle/Contents/Resources/licenses/NONO-APACHE-2.0.txt"

identity=${CODESIGN_IDENTITY:--}
sign() {
    if [ "$identity" = "-" ]; then
        codesign --force --sign - "$1"
    else
        codesign --force --options runtime --timestamp --sign "$identity" "$1"
    fi
}
sign "$bundle/Contents/MacOS/nono"
sign "$bundle/Contents/MacOS/farcaster"
sign "$bundle"
codesign --verify --deep --strict "$bundle"

printf '%s\n' "$bundle"
