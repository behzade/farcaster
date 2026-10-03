#!/usr/bin/env bash
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
binary=$(realpath "$target_dir/release/farcaster")
out=$(realpath "${1:?usage: package-appimage.sh OUTPUT_DIRECTORY}")
command -v linuxdeploy >/dev/null || {
    echo "Install official linuxdeploy and add it to PATH before packaging an AppImage" >&2
    exit 1
}
version=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "farcaster"))')
arch=$(uname -m)
stage=$(mktemp -d "$out/appimage.XXXXXX")
trap 'rm -rf "$stage"' EXIT
appdir="$stage/AppDir"
sh scripts/install-linux.sh "$binary" "$appdir/usr"

xcb_libdir=$(pkg-config --variable=libdir xcb)
wayland_libdir=$(pkg-config --variable=libdir wayland-client)
vulkan_libdir=$(pkg-config --variable=libdir vulkan)
egl_libdir=$(pkg-config --variable=libdir egl)
libraries=()
for library in "$xcb_libdir/libxcb.so.1" "$wayland_libdir/libwayland-egl.so.1" \
    "$vulkan_libdir/libvulkan.so.1" "$egl_libdir/libEGL.so.1" \
    "$egl_libdir/libGLdispatch.so.0"; do
    if [ ! -f "$library" ]; then
        echo "Missing AppImage runtime library: $library" >&2
        exit 1
    fi
    libraries+=(--library "$library")
done

filename="Farcaster-v${version}-${arch}.AppImage"
candidate="$stage/$filename"
unset SOURCE_DATE_EPOCH
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" VERSION="$version" OUTPUT="$candidate" \
    linuxdeploy --appdir "$appdir" "${libraries[@]}" \
    --exclude-library 'libwayland-client.so*' --output appimage

(
    cd "$stage"
    # NixOS binfmt launches AppRun, passing runtime flags to the application.
    # Use its runner's extraction mode instead of executing the image there.
    if command -v appimage-run >/dev/null; then
        env -u APPIMAGE_EXTRACT_AND_RUN appimage-run -x "$PWD/squashfs-root" "$candidate" > /dev/null
    else
        env -u APPIMAGE_EXTRACT_AND_RUN "$candidate" --appimage-extract > /dev/null
    fi
    for executable in squashfs-root/AppRun squashfs-root/usr/bin/farcaster; do
        if [ ! -f "$executable" ]; then
            echo "Missing AppImage executable: $executable" >&2
            exit 1
        fi
        mode=$(stat -Lc '%a' "$executable")
        if (( (8#$mode & 0111) != 0111 )); then
            echo "AppImage executable needs execute permission for all users: $executable ($mode)" >&2
            exit 1
        fi
    done
    if [ -n "$(find squashfs-root -name 'libwayland-client.so*' -print -quit)" ]; then
        echo "AppImage must use the host libwayland-client" >&2
        exit 1
    fi
)
mv "$candidate" "$out/$filename"
