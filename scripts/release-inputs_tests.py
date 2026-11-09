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
                         [version, version])

    def test_linux_runtime_matches_ghostty(self):
        for workflow in ("linux-packages", "first-party-tests"):
            with self.subTest(workflow=workflow):
                source = (ROOT / f".github/workflows/{workflow}.yml").read_text()
                self.assertIn("llvm-toolchain-jammy-22 main", source)
                self.assertIn("libc++-22-dev libc++abi-22-dev", source)
        arch = (ROOT / "packaging/arch/PKGBUILD.in").read_text()
        self.assertIn("'libc++>=22'", arch)
        for path in ("flake.nix", "packaging/nix.nix"):
            with self.subTest(path=path):
                self.assertIn("llvmPackages_22.libcxx", (ROOT / path).read_text())

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
