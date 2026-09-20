#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stamp=$root/target/.prereqs
zig=${ZIG:-}
[ -n "$zig" ] || zig=$(command -v zig 2>/dev/null || true)

die() { echo "$*" >&2; exit 1; }

command -v cargo >/dev/null 2>&1 || die "cargo is missing"
[ -x "$zig" ] || die "zig is missing"

checked=
[ -f "$stamp" ] && read -r checked <"$stamp" || true
if [ "$zig" != "$checked" ] || [ "$zig" -nt "$stamp" ]; then
    version=$("$zig" version 2>/dev/null || true)
    minor=${version#*.}
    minor=${minor%%.*}
    [ "$minor" -ge 16 ] 2>/dev/null || [ "${version%%.*}" -gt 0 ] 2>/dev/null ||
        die "zig 0.16 or newer is required, found ${version:-nothing}"
    if [ "$(uname -s)" = Darwin ]; then
        xcrun --sdk macosx --show-sdk-path >/dev/null 2>&1 ||
            die "the Xcode command line tools are missing, install them with: xcode-select --install"
    fi
    mkdir -p "$root/target" 2>/dev/null || true
    printf '%s\n' "$zig" >"$stamp" 2>/dev/null || true
fi

if [ "$#" -gt 0 ]; then
    exec "$@"
fi
