#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("bundle.sh")
LIBRARIES = ("libxcb.so.1", "libwayland-egl.so.1", "libvulkan.so.1",
             "libEGL.so.1", "libGLdispatch.so.0")
TOOLS = r"""
import json, os, pathlib, shutil, sys
root = pathlib.Path(os.environ["PACKAGE_FIXTURE"])
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
if name == "cargo":
    if args[0] == "build":
        (root / "build.json").write_text(json.dumps(args))
    else:
        assert args == ["metadata", "--no-deps", "--format-version", "1"]
        print('{"packages":[{"name":"other","version":"9.9.9"},{"name":"farcaster","version":"1.2.3"}]}')
elif name == "pkg-config":
    print(root / "lib")
elif name == "uname":
    print("Linux" if args == ["-s"] else "x86_64")
elif name == "linuxdeploy":
    appdir = pathlib.Path(args[args.index("--appdir") + 1])
    (root / "deploy.json").write_text(json.dumps(args))
    for i, arg in enumerate(args):
        if arg == "--library":
            source = pathlib.Path(args[i + 1])
            destination = appdir / "usr/lib" / source.name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
    (appdir / "AppRun").symlink_to("usr/bin/farcaster")
    shutil.copytree(appdir, root / "image", symlinks=True)
    output = pathlib.Path(os.environ["OUTPUT"])
    shutil.copyfile(__file__, output)
    output.chmod(0o755)
    if json.loads((root / "settings.json").read_text()).get("deploy_failure"):
        sys.exit(1)
elif name == "appimage-run" or name.endswith(".AppImage"):
    settings = json.loads((root / "settings.json").read_text())
    assert "APPIMAGE_EXTRACT_AND_RUN" not in os.environ
    if name == "appimage-run":
        assert args[0] == "-x" and len(args) == 3
        assert pathlib.Path(args[2]).is_file()
        appdir = pathlib.Path(args[1])
        assert appdir.is_absolute()
    else:
        if settings.get("runner"):
            sys.exit("resolve project --appimage-extract: No such file or directory")
        assert args == ["--appimage-extract"]
        appdir = pathlib.Path("squashfs-root")
    (root / "extract.json").write_text(json.dumps({"tool": name, "args": args}))
    if settings.get("extract_failure"):
        sys.exit(1)
    shutil.copytree(root / "image", appdir, symlinks=True)
    if settings.get("apprun_mode") is not None:
        (appdir / "AppRun").unlink()
        (appdir / "AppRun").write_text("fixture launcher")
        (appdir / "AppRun").chmod(settings["apprun_mode"])
    if settings.get("binary_mode") is not None:
        (appdir / "usr/bin/farcaster").chmod(settings["binary_mode"])
    if settings.get("broken_apprun"):
        (appdir / "AppRun").unlink()
        (appdir / "AppRun").symlink_to("missing")
    if settings.get("bundled_wayland"):
        (appdir / "usr/lib/libwayland-client.so.0").write_text("wrong host ABI")
else:
    raise AssertionError(name)
"""


class AppImagePackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="farcaster appimage ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for directory in ("tools", "release", "lib"):
            (self.root / directory).mkdir()
        (self.root / "release/farcaster").write_text("fixture release binary")
        for library in LIBRARIES:
            (self.root / "lib" / library).write_text("fixture " + library)
        for name in ("cargo", "pkg-config", "uname", "linuxdeploy"):
            tool = self.root / "tools" / name
            tool.write_text(f"#!{sys.executable}\n" + TOOLS)
            tool.chmod(0o755)
        # Do not accidentally use the host's appimage-run in native-runtime tests.
        for name in ("bash", "sh", "env", "dirname", "realpath", "mkdir", "mktemp",
                     "install", "cp", "stat", "find", "mv", "rm", "python3"):
            (self.root / "tools" / name).symlink_to(shutil.which(name))
        self.output = self.root / "release/Farcaster-v1.2.3-x86_64.AppImage"

    def package(self, **settings):
        (self.root / "settings.json").write_text(json.dumps(settings))
        if settings.get("runner"):
            (self.root / "tools/appimage-run").symlink_to("linuxdeploy")
        environment = dict(os.environ, PACKAGE_FIXTURE=str(self.root),
                           PATH=str(self.root / "tools"),
                           CARGO_TARGET_DIR=str(self.root), BUNDLE_FORMATS="appimage",
                           APPIMAGE_EXTRACT_AND_RUN="1")
        return subprocess.run(["sh", str(SCRIPT)],
                              env=environment, capture_output=True, text=True)

    def test_bundle_stages_assets_and_packages_runtime_libraries(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads((self.root / "build.json").read_text()),
                         ["build", "--release", "--locked", "--bin", "farcaster"])
        self.assertTrue(self.output.is_file())
        self.assertEqual({p.name for p in self.output.parent.iterdir()},
                         {"farcaster", self.output.name})

        staged = self.root / "image"
        self.assertEqual((staged / "usr/bin/farcaster").stat().st_mode & 0o777, 0o755)
        self.assertEqual({p.name for p in (staged / "usr/lib").iterdir()}, set(LIBRARIES))
        self.assertIn("Exec=farcaster %f", (staged / "usr/share/applications/io.github.behzade.farcaster.desktop").read_text())
        self.assertTrue((staged / "usr/share/icons/hicolor/256x256/apps/io.github.behzade.farcaster.png").is_file())
        self.assertTrue((staged / "usr/share/licenses/farcaster/NOTICE.md").is_file())
        args = json.loads((self.root / "deploy.json").read_text())
        self.assertEqual(args[args.index("--exclude-library") + 1], "libwayland-client.so*")

    def test_extracts_with_nixos_runner_without_launching_application(self):
        result = self.package(runner=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.output.is_file())
        extraction = json.loads((self.root / "extract.json").read_text())
        self.assertEqual(extraction["tool"], "appimage-run")
        self.assertEqual(extraction["args"][0], "-x")

    def test_runner_extraction_still_checks_permissions(self):
        result = self.package(runner=True, binary_mode=0o764)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("execute permission for all users", result.stderr)
        self.assertFalse(self.output.exists())

    def test_runner_extraction_still_rejects_bundled_host_wayland(self):
        result = self.package(runner=True, bundled_wayland=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("host libwayland-client", result.stderr)
        self.assertFalse(self.output.exists())

    def test_runner_extraction_failure_preserves_previous_image(self):
        self.output.write_text("previous image")
        result = self.package(runner=True, extract_failure=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(json.loads((self.root / "extract.json").read_text())["tool"],
                         "appimage-run")
        self.assertEqual(self.output.read_text(), "previous image")
        self.assertEqual({p.name for p in self.output.parent.iterdir()},
                         {"farcaster", self.output.name})

    def test_rejects_owner_only_execution_in_finished_image(self):
        result = self.package(apprun_mode=0o744)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("execute permission for all users", result.stderr)
        self.assertFalse(self.output.exists())

    def test_checks_symlink_target_permissions(self):
        result = self.package(binary_mode=0o764)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("execute permission for all users", result.stderr)
        self.assertFalse(self.output.exists())

    def test_rejects_broken_apprun(self):
        self.assertNotEqual(self.package(broken_apprun=True).returncode, 0)
        self.assertFalse(self.output.exists())

    def test_rejects_bundled_host_wayland(self):
        result = self.package(bundled_wayland=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("host libwayland-client", result.stderr)
        self.assertFalse(self.output.exists())

    def test_deploy_failure_does_not_publish_partial_image(self):
        self.assertNotEqual(self.package(deploy_failure=True).returncode, 0)
        self.assertEqual(list((self.root / "release").iterdir()),
                         [self.root / "release/farcaster"])


if __name__ == "__main__":
    unittest.main()
