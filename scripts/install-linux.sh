#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:?usage: install-linux.sh BINARY PREFIX}
prefix=${2:?usage: install-linux.sh BINARY PREFIX}
install -Dm755 "$binary" "$prefix/bin/farcaster"
install -Dm644 "$root/packaging/io.github.behzade.farcaster.desktop" \
    "$prefix/share/applications/io.github.behzade.farcaster.desktop"
for size in 16 32 128 256 512; do
    install -Dm644 "$root/assets/icons/app/icon_${size}x${size}.png" \
        "$prefix/share/icons/hicolor/${size}x${size}/apps/io.github.behzade.farcaster.png"
done
licenses="$prefix/share/licenses/farcaster"
install -Dm644 "$root/LICENSE" "$licenses/LICENSE"
install -m644 "$root/NOTICE.md" "$licenses/NOTICE.md"
cp -R "$root/THIRD_PARTY_LICENSES" "$licenses/"
install -m644 "$root/assets/ibm-plex-sans/OFL.txt" "$licenses/THIRD_PARTY_LICENSES/IBM-PLEX-OFL.txt"
install -m644 "$root/assets/lilex/OFL.txt" "$licenses/THIRD_PARTY_LICENSES/LILEX-OFL.txt"
install -m644 "$root/assets/vazirmatn/OFL.txt" "$licenses/THIRD_PARTY_LICENSES/VAZIRMATN-OFL.txt"
install -m644 "$root/third_party/gpui-component-bd83329/LICENSE-APACHE" "$licenses/THIRD_PARTY_LICENSES/GPUI-COMPONENT-APACHE-2.0.txt"
