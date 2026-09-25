# Dependency hashes from Ghostty 9f0e1719dc918368367d368bfe300f59bb68b5a4
# build.zig.zon.nix (MIT), matching gpui-libghostty 0.3.1's vendored source.
{
  lib,
  fetchurl,
  fetchzip,
  fetchgit,
  runCommand,
  zig_0_16,
  zstd,
}:
let
  manifest = builtins.fromJSON (builtins.readFile ./ghostty-zig-deps.json);
  fetchDependency =
    zigHash: dep:
    let
      isGit = lib.hasPrefix "git+" dep.url;
      gitParts = lib.splitString "#" (lib.removePrefix "git+" dep.url);
      artifact =
        if isGit then
          fetchgit {
            url = builtins.head gitParts;
            rev = builtins.elemAt gitParts 1;
            fetchSubmodules = false;
            deepClone = false;
            inherit (dep) hash;
          }
        else if dep.unpack then
          fetchzip {
            inherit (dep) url hash;
            nativeBuildInputs = [ zstd ];
          }
        else
          fetchurl {
            inherit (dep) url hash;
          };
    in
    runCommand "ghostty-zig-${dep.name}" { nativeBuildInputs = [ zig_0_16 ]; } ''
      mkdir -p source cache/tmp
      touch source/build.zig
      cd source
      actual=$(zig fetch --global-cache-dir "$NIX_BUILD_TOP/cache" ${artifact})
      test "$actual" = ${lib.escapeShellArg zigHash}
      mkdir "$out"
      tar xzf "$NIX_BUILD_TOP/cache/p/$actual.tar.gz" -C "$out" --strip-components=1
    '';
  packages = lib.mapAttrs fetchDependency manifest;
in
runCommand "farcaster-ghostty-zig-deps" { } (
  ''mkdir -p "$out"''
  + "\n"
  + lib.concatStringsSep "\n" (
    lib.mapAttrsToList (name: path: ''
      # Real directories keep Zig's relative build paths and header scans valid.
      cp -R --no-preserve=mode ${path} "$out/${name}"
    '') packages
  )
)
