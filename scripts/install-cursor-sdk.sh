#!/bin/sh
# Install the official bridge and SDK used for live runs and browser sign-in.
set -eu
version=1.0.35
command -v npm >/dev/null 2>&1 || { echo 'Install Node.js 22.13+ and npm for the Cursor SDK' >&2; exit 1; }
node -e 'const [major, minor] = process.versions.node.split(".").map(Number); process.exit(major > 22 || (major === 22 && minor >= 13) ? 0 : 1)' || {
    echo 'Cursor SDK requires Node.js 22.13+' >&2; exit 1;
}
case "$(uname -s)" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    *) echo 'Cursor SDK installer supports macOS and Linux' >&2; exit 1 ;;
esac
case "$(uname -m)" in
    arm64|aarch64) arch=arm64 ;;
    x86_64|amd64) arch=x64 ;;
    *) echo 'Unsupported Cursor SDK architecture' >&2; exit 1 ;;
esac
install_dir=${1:-"$HOME/.local/share/farcaster/cursor-sdk/$version"}
archive="cursor-sdk-bridge-standalone-$os-$arch.tar.gz"
base="https://github.com/cursor/sdk-bridge/releases/download/v$version"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
curl --fail --location --silent --show-error "$base/$archive" -o "$tmp/$archive"
curl --fail --location --silent --show-error "$base/SHA256SUMS.txt" -o "$tmp/SHA256SUMS.txt"
expected=$(awk -v file="$archive" '$2 == file { print $1 }' "$tmp/SHA256SUMS.txt")
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$archive" | awk '{print $1}')
else
    actual=$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')
fi
[ -n "$expected" ] && [ "$actual" = "$expected" ] || { echo 'Cursor SDK checksum mismatch' >&2; exit 1; }
mkdir -p "$install_dir"
tar -xzf "$tmp/$archive" -C "$install_dir"
# The bridge resolves native helpers from this node_modules directory.
npm install --prefix "$install_dir" --ignore-scripts --no-audit --no-fund --save-exact "@cursor/sdk@$version"
[ -x "$install_dir/node_modules/@cursor/sdk-$os-$arch/bin/cursorsandbox" ] || {
    echo 'Cursor SDK sandbox helper is missing from the installation' >&2; exit 1;
}
printf 'Installed Cursor SDK bridge: %s/bin/cursor-sdk-bridge\n' "$install_dir"
