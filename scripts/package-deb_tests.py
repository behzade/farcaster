#!/usr/bin/env python3
"""Exercise the packaging script with fixture linker/package metadata, without a Rust build.

System-package tools are boundary doubles; staging, copying, and control-file
creation use the real scripts. Native package installation stays covered by CI.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("package-deb.sh")
TOOLS = r'''
import json, os, pathlib, shutil, sys
root = pathlib.Path(os.environ["PACKAGE_FIXTURE"])
config = json.loads((root / "metadata.json").read_text())
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if name == "cargo":
    assert args == ["metadata", "--no-deps", "--format-version", "1"]
    print('{"packages":[{"version":"1.2.3"}]}')
elif name == "dpkg":
    assert args == ["--print-architecture"]
    print("amd64")
elif name == "ldd":
    print(config["ldd"])
elif name == "dpkg-query":
    assert args[0] in ("-S", "--search")
    owner = config["owners"].get(args[-1])
    if owner is None:
        sys.exit(1)
    print(owner + ": " + args[-1])
elif name == "install":
    args = [str(root / "docs" / arg.removeprefix("/usr/share/doc/"))
            if arg.startswith("/usr/share/doc/") else arg for arg in args]
    os.execv(os.environ["REAL_INSTALL"], ["install", *args])
elif name == "patchelf":
    assert args[0] == "--set-rpath"
    assert pathlib.Path(args[2]).is_file()
    with (root / "rpaths.jsonl").open("a") as log:
        log.write(json.dumps(args[1:]) + "\n")
elif name == "dpkg-shlibdeps":
    assert all(pathlib.Path(arg[2:]).is_file() for arg in args if arg.startswith("-e"))
    (root / "shlibs").write_text(pathlib.Path("debian/shlibs.local").read_text())
    (root / "shlibdeps.json").write_text(json.dumps(args))
    print("shlibs:Depends=libc6 (>= 2.35), libgcc-s1 (>= 4.2)")
elif name == "dpkg-deb":
    assert args[:2] == ["--root-owner-group", "--build"]
    shutil.copytree(args[2], args[3])
else:
    raise AssertionError(name)
'''


class DebianPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="farcaster-package-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for directory in ("tools", "release", "out", "tmp", "lib", "docs"):
            (self.root / directory).mkdir()
        (self.root / "release/farcaster").write_text("fixture release binary")
        self.metadata = {"ldd": "", "owners": {}}
        self.entries = {}
        for soname, owner in (("libc++.so.1", "libc++1:amd64"),
                              ("libc++abi.so.1", "libc++abi1:amd64")):
            self.add_library(soname, owner)
        for name in ("cargo", "dpkg", "ldd", "dpkg-query", "install", "patchelf",
                     "dpkg-shlibdeps", "dpkg-deb"):
            tool = self.root / "tools" / name
            tool.write_text(f"#!{sys.executable}\n" + TOOLS)
            tool.chmod(0o755)
        self.package = self.root / "out/farcaster_1.2.3_amd64.deb"

    def add_library(self, soname, owner):
        library = self.root / "lib" / (soname + ".0")
        library.write_text("fixture " + soname)
        link = self.root / "lib" / soname
        link.symlink_to(library.name)
        self.entries[soname] = f"{soname} => {link} (0x1234)"
        self.metadata["owners"][str(library)] = owner
        self.add_copyright(owner)

    def add_copyright(self, owner):
        path = self.root / "docs" / owner.split(":")[0] / "copyright"
        path.parent.mkdir(exist_ok=True)
        path.write_text("license for " + owner)

    def package_release(self):
        self.metadata["ldd"] = "\n".join(self.entries.values())
        (self.root / "metadata.json").write_text(json.dumps(self.metadata))
        environment = dict(os.environ, PACKAGE_FIXTURE=str(self.root),
                           REAL_INSTALL=shutil.which("install"),
                           PATH=str(self.root / "tools") + os.pathsep + os.environ["PATH"],
                           CARGO_TARGET_DIR=str(self.root), TMPDIR=str(self.root / "tmp"))
        return subprocess.run(["bash", str(SCRIPT), str(self.root / "out")],
                              env=environment, capture_output=True, text=True)

    def assert_packaged(self, sonames, owners):
        result = self.package_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        private = self.package / "usr/lib/farcaster/lib"
        self.assertEqual({p.name for p in private.iterdir()}, set(sonames))
        for soname in sonames:
            self.assertEqual((private / soname).read_text(), "fixture " + soname)
        licenses = self.package / "usr/share/licenses/farcaster"
        for owner in owners:
            self.assertEqual((licenses / (owner.split(":")[0] + "-copyright")).read_text(),
                             "license for " + owner)
        shlibs = (self.root / "shlibs").read_text().splitlines()
        expected = {name.split(".so.")[0] + " 1 farcaster" for name in sonames}
        self.assertEqual(set(shlibs), expected)
        arguments = json.loads((self.root / "shlibdeps.json").read_text())
        self.assertIn("-xfarcaster", arguments)
        inputs = [Path(arg[2:]) for arg in arguments if arg.startswith("-e")]
        self.assertEqual({path.name for path in inputs}, {*sonames, "farcaster"})
        binary = next(path for path in inputs if path.name == "farcaster")
        self.assertIn("-l" + str(binary.parent / "lib"), arguments)
        rpaths = [json.loads(line) for line in (self.root / "rpaths.jsonl").read_text().splitlines()]
        self.assertEqual({(value, Path(path).name) for value, path in rpaths},
                         {("$ORIGIN", name) for name in sonames} | {("$ORIGIN/lib", "farcaster")})
        self.assertIn("libgcc-s1 (>= 4.2)", (self.package / "DEBIAN/control").read_text())

    def test_ci_runtime_without_llvm_unwind(self):
        self.assert_packaged(["libc++.so.1", "libc++abi.so.1"],
                             ["libc++1:amd64", "libc++abi1:amd64"])

    def test_unversioned_packages_with_linked_unwind(self):
        for path, owner in list(self.metadata["owners"].items()):
            owner = owner.split(":")[0]
            self.metadata["owners"][path] = owner
            self.add_copyright(owner)
        self.add_library("libunwind.so.1", "libunwind-21:amd64")
        self.assert_packaged(["libc++.so.1", "libc++abi.so.1", "libunwind.so.1"],
                             ["libc++1", "libc++abi1", "libunwind-21:amd64"])

    def test_linked_unwind_and_versioned_packages(self):
        for path, owner in list(self.metadata["owners"].items()):
            owner = owner.replace(":amd64", "-21:amd64")
            self.metadata["owners"][path] = owner
            self.add_copyright(owner)
        self.add_library("libunwind.so.1", "libunwind-21:amd64")
        self.assert_packaged(["libc++.so.1", "libc++abi.so.1", "libunwind.so.1"],
                             ["libc++1-21:amd64", "libc++abi1-21:amd64", "libunwind-21:amd64"])

    def test_unresolved_unwind_is_not_optional(self):
        self.entries["libunwind.so.1"] = "libunwind.so.1 => not found"
        result = self.package_release()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("libunwind.so.1", result.stderr)
        self.assertFalse(self.package.exists())

    def test_required_runtime_absent_or_unresolved(self):
        for soname in list(self.entries):
            original = self.entries[soname]
            for entry in (None, soname + " => not found"):
                with self.subTest(soname=soname, entry=entry):
                    self.entries.pop(soname, None)
                    if entry:
                        self.entries[soname] = entry
                    result = self.package_release()
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(soname, result.stderr)
                    self.assertFalse(self.package.exists())
            self.entries[soname] = original

    def test_unknown_or_ambiguous_package_owner_fails(self):
        path = next(iter(self.metadata["owners"]))
        for owner in (None, "libc++1:amd64, other-package:amd64",
                      f"libc++1:amd64: {path}\nother-package:amd64"):
            with self.subTest(owner=owner):
                self.metadata["owners"][path] = owner
                self.assertNotEqual(self.package_release().returncode, 0)
                self.assertFalse(self.package.exists())

    def test_missing_copyright_fails(self):
        (self.root / "docs/libc++1/copyright").unlink()
        self.assertNotEqual(self.package_release().returncode, 0)
        self.assertFalse(self.package.exists())


if __name__ == "__main__":
    unittest.main()
