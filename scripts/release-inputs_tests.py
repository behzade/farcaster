#!/usr/bin/env python3
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parent.parent


class ReleaseInputsTests(unittest.TestCase):
    def test_zig_versions_match_across_builds(self):
        arch = (ROOT / "packaging/arch/PKGBUILD.in").read_text()
        version = re.search(r"^_zigver=(.+)$", arch, re.MULTILINE).group(1)
        self.assertEqual(version, "0.17.0")
        for workflow in ("release", "linux-packages", "first-party-tests"):
            with self.subTest(workflow=workflow):
                source = (ROOT / f".github/workflows/{workflow}.yml").read_text()
                versions = re.findall(r"setup-zig@[^\n]+\n\s+with:\n\s+version: ([^\n]+)", source)
                self.assertEqual(versions, [version])
        flake = (ROOT / "flake.nix").read_text()
        self.assertEqual(re.findall(r'zig-overlay\.packages\.\$\{system\}\."([^"]+)"', flake),
                         [version])
        self.assertIn("zig = zigFor system;", flake)
        self.assertIn("(zigFor system)", flake)

    def test_linux_runtime_matches_ghostty(self):
        for workflow in ("linux-packages", "first-party-tests"):
            with self.subTest(workflow=workflow):
                source = (ROOT / f".github/workflows/{workflow}.yml").read_text()
                self.assertIn("llvm-toolchain-jammy-22 main", source)
                self.assertIn("libc++-22-dev libc++abi-22-dev", source)
        arch = (ROOT / "packaging/arch/PKGBUILD.in").read_text()
        self.assertIn("'libc++>=22'", arch)
        flake = (ROOT / "flake.nix").read_text()
        for path in ("flake.nix", "packaging/nix.nix"):
            with self.subTest(path=path):
                self.assertIn("llvmPackages_22.libcxx", (ROOT / path).read_text())

    def test_nix_uses_the_pinned_llvm_toolchain(self):
        flake = (ROOT / "flake.nix").read_text()
        self.assertNotIn("libcxxFor", flake)
        self.assertNotRegex(flake, r"libcxx\.override\b")
        self.assertNotIn("llvmPackages_21", flake)

    def test_nix_preserves_native_zig_target(self):
        flake = (ROOT / "flake.nix").read_text()
        self.assertIn("installPhase = previous.installPhase + ''", flake)
        self.assertNotIn('postInstall = (previous.postInstall or "")', flake)
        self.assertIn('patch -d "$out" -p1 < ${./packaging/zig-native-target.patch}', flake)
        self.assertIn('nixpkgs.lib.hasSuffix "-linux" system', flake)
        patch = (ROOT / "packaging/zig-native-target.patch").read_text()
        for fallback in ("rt.query.get(c) orelse resolved",
                         "orelse resolved.flags.os_tag.unwrap()",
                         "orelse resolved.flags.abi.unwrap()",
                         "orelse resolved.dynamic_linker.value",
                         "try arena.dupe(u8, dynamic_linker_path)"):
            with self.subTest(fallback=fallback):
                self.assertIn(fallback, patch)

    def test_release_commit_uses_package_version(self):
        config = (ROOT / "release.toml").read_text()
        self.assertRegex(config, r"(?m)^consolidate-commits = false$")
        self.assertIn('pre-release-commit-message = "chore(release): prepare v{{version}}"',
                      config)

    def test_nix_builds_linux_ghostty_shim_with_pic(self):
        source = (ROOT / "packaging/nix.nix").read_text()
        self.assertIn('postPatch = \'\'', source)
        self.assertIn('patch -d "$cargoDepsCopy/gpui-libghostty-${ghostty.version}" '
                      '-p1 < ${./ghostty-linux-shim-pic.patch}', source)
        patch = (ROOT / "packaging/ghostty-linux-shim-pic.patch").read_text()
        self.assertIn("fn compile_linux_shim(", patch)
        additions = [line[1:] for line in patch.splitlines()
                     if line.startswith("+") and not line.startswith("+++")]
        self.assertEqual(additions, [
            '        .args(["cc", "-std=c11", "-D_POSIX_C_SOURCE=200809L", '
            '"-fPIC", "-c", "-I"])',
        ])

    def test_nix_guard_matches_locked_ghostty(self):
        lock = (ROOT / "Cargo.lock").read_text()
        version = re.search(r'name = "gpui-libghostty"\nversion = "([^"]+)"', lock).group(1)
        source = (ROOT / "packaging/nix.nix").read_text()
        self.assertIn(f'ghostty.version == "{version}"', source)
        dependencies = (ROOT / "packaging/ghostty-zig-deps.nix").read_text()
        self.assertIn(f"gpui-libghostty {version}'s upstream source", dependencies)

    def test_nix_shares_the_release_compiler(self):
        source = (ROOT / "packaging/nix.nix").read_text()
        self.assertIn("callPackage ./ghostty-zig-deps.nix { inherit zig; }", source)
        self.assertIn('${zig}/bin/zig build', source)
        dependencies = (ROOT / "packaging/ghostty-zig-deps.nix").read_text()
        self.assertIn("nativeBuildInputs = [ zig ];", dependencies)
        self.assertIn('export ZIG_GLOBAL_CACHE_DIR="$NIX_BUILD_TOP/cache"', dependencies)
        self.assertIn('export ZIG_LOCAL_CACHE_DIR="$NIX_BUILD_TOP/zig-cache"', dependencies)
        self.assertNotIn("--global-cache-dir", dependencies)
        self.assertIn('tar xzf "$ZIG_GLOBAL_CACHE_DIR/p/$actual.tar.gz" -C "$out"', dependencies)
        self.assertNotIn("zig_0_16", source + dependencies)


if __name__ == "__main__":
    unittest.main()
