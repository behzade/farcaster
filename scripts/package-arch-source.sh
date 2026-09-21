#!/usr/bin/env bash
# Stage a source archive and a checksummed PKGBUILD for the exact release commit.
set -euo pipefail
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
out=$(realpath "${1:?usage: package-arch-source.sh OUTPUT_DIRECTORY}")
version=$(git show HEAD:Cargo.toml | sed -n 's/^version = "\([^"]*\)"$/\1/p')
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
if ! git cat-file -e HEAD:scripts/install-linux.sh; then
    echo 'Arch source packages require the packaging changes in the commit being archived.' >&2
    exit 1
fi
git archive --format=tar --prefix="farcaster-$version/" HEAD | gzip -n > "$out/farcaster-$version-source.tar.gz"
checksum=$(sha256sum "$out/farcaster-$version-source.tar.gz" | cut -d ' ' -f 1)
sed -e "s/@VERSION@/$version/g" -e "s/@SHA256@/$checksum/g" \
    packaging/arch/PKGBUILD.in > "$out/PKGBUILD"
