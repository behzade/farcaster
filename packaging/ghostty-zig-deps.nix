# Dependency hashes from Ghostty 83fb95624b0439ee660c2679b43b38c8b63769cd
# build.zig.zon.nix (MIT), matching gpui-libghostty 0.4.1-rc.1's upstream source.
{
  lib,
  fetchurl,
  fetchzip,
  fetchgit,
  runCommand,
  zig,
  zstd,
}:
let
  manifest = builtins.fromJSON (builtins.readFile ./ghostty-zig-deps.json);
  fetchDependency =
    zigHash: dep:
    let
      isGit = lib.hasPrefix "git+" dep.url;
      gitParts = lib.splitString "#" (lib.removePrefix "git+" dep.url);
    in
    if isGit then
      fetchgit {
        url = builtins.head gitParts;
        rev = builtins.elemAt gitParts 1;
        fetchSubmodules = false;
        deepClone = false;
        preFetch = ''
          export GIT_ATTR_SOURCE=4b825dc642cb6eb9a060e54bf8d69288fbee4904
          export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.autocrlf GIT_CONFIG_VALUE_0=false
        '';
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
  packages = lib.mapAttrs fetchDependency manifest;
in
runCommand "farcaster-ghostty-zig-deps" { nativeBuildInputs = [ zig ]; } (
  ''
    mkdir -p "$out" cache/tmp
    export ZIG_GLOBAL_CACHE_DIR="$NIX_BUILD_TOP/cache"
    export ZIG_LOCAL_CACHE_DIR="$NIX_BUILD_TOP/zig-cache"
  ''
  + "\n"
  + lib.concatStringsSep "\n" (
    lib.mapAttrsToList (zigHash: path: ''
      actual=$(zig fetch ${path})
      test "$actual" = ${lib.escapeShellArg zigHash}
      tar xzf "$ZIG_GLOBAL_CACHE_DIR/p/$actual.tar.gz" -C "$out"
      rm "$ZIG_GLOBAL_CACHE_DIR/p/$actual.tar.gz"
    '') packages
  )
)
