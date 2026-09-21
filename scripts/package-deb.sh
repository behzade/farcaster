#!/usr/bin/env bash
# Package the release binary built on Ubuntu 22.04. No build tools at runtime.
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
binary=$(realpath "$target_dir/release/farcaster")
out=$(realpath "${1:?usage: package-deb.sh OUTPUT_DIRECTORY}")
version=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
arch=$(dpkg --print-architecture)
stage=$(mktemp -d)
# Retain staging on failure for inspection. CI owns its temporary filesystem.
package_root="$stage/package"
sh scripts/install-linux.sh "$binary" "$package_root/usr"
mkdir -p "$package_root/usr/lib/farcaster/lib" "$package_root/DEBIAN" "$stage/debian"
mv "$package_root/usr/bin/farcaster" "$package_root/usr/lib/farcaster/farcaster"
ln -s ../lib/farcaster/farcaster "$package_root/usr/bin/farcaster"

# Ghostty needs LLVM 21's runtime. Keep it private so installation needs no
# third-party apt repository and cannot replace another application's libc++.
libraries=()
linked_libraries=$(ldd "$binary")
for soname in libc++.so.1 libc++abi.so.1 libunwind.so.1; do
    library=$(awk -v name="$soname" '$1 == name && $2 == "=>" { print $3; exit }' <<< "$linked_libraries")
    if [ -z "$library" ]; then
        echo "Missing expected LLVM runtime: $soname" >&2
        exit 1
    fi
    install -m755 "$library" "$package_root/usr/lib/farcaster/lib/$soname"
    patchelf --set-rpath "\$ORIGIN" "$package_root/usr/lib/farcaster/lib/$soname"
    libraries+=("-e$package_root/usr/lib/farcaster/lib/$soname")
done
patchelf --set-rpath "\$ORIGIN/lib" "$package_root/usr/lib/farcaster/farcaster"
for package in libc++1-21 libc++abi1-21 libunwind-21; do
    install -m644 "/usr/share/doc/$package/copyright" "$package_root/usr/share/licenses/farcaster/$package-copyright"
done

# dpkg derives versioned dependencies from the ELF symbols, including those
# used by the private runtimes. Only those three bundled packages are excluded.
printf 'Source: farcaster\nSection: devel\nPriority: optional\nMaintainer: Farcaster contributors <noreply@github.com>\n\nPackage: farcaster\nArchitecture: any\nDescription: Native desktop client for coding agents\n' > "$stage/debian/control"
# Tell shlibdeps which package supplies the private libraries, then omit the
# resulting self-dependency. Other libraries still require real symbol metadata.
printf 'libc++ 1 farcaster\nlibc++abi 1 farcaster\nlibunwind 1 farcaster\n' > "$stage/debian/shlibs.local"
dependencies=$(
    cd "$stage"
    dpkg-shlibdeps -O -e"$package_root/usr/lib/farcaster/farcaster" "${libraries[@]}" \
        -l"$package_root/usr/lib/farcaster/lib" -xfarcaster
)
dependencies=${dependencies#shlibs:Depends=}
# GPUI loads these graphics libraries at runtime, beyond ELF's NEEDED entries.
dependencies="$dependencies, libegl1, libgl1, libvulkan1, libwayland-client0, libwayland-egl1, libxkbcommon0, libxkbcommon-x11-0, libxcb1, fontconfig, git, ca-certificates"
printf 'Package: farcaster\nVersion: %s\nArchitecture: %s\nMaintainer: Farcaster contributors <noreply@github.com>\nSection: devel\nPriority: optional\nDepends: %s\nRecommends: neovim\nHomepage: https://github.com/behzade/farcaster\nDescription: Native desktop client for coding agents\n' \
    "$version" "$arch" "$dependencies" > "$package_root/DEBIAN/control"
dpkg-deb --root-owner-group --build "$package_root" "$out/farcaster_${version}_${arch}.deb"
