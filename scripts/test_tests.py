#!/usr/bin/env python3

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = REPO_ROOT / "scripts/test.sh"
INVENTORY = REPO_ROOT / "scripts/first-party-packages.txt"
PRESERVED_ENV = ("HOME", "CARGO_HOME", "CARGO_TARGET_DIR", "RUSTUP_HOME", "XDG_CACHE_HOME")
MOCK_CARGO = r'''
import json, os, pathlib, sys
data = pathlib.Path(os.environ["FARCASTER_DATA_DIR"])
assert data.is_dir()
(data / "fixture").write_text("isolated test data")
with open(os.environ["TEST_CARGO_LOG"], "a") as log:
    log.write(json.dumps({
        "args": sys.argv[1:],
        "cwd": os.getcwd(),
        "data": str(data),
        "stdin": sys.stdin.read(),
        "environment": {key: os.environ.get(key) for key in
            ("HOME", "CARGO_HOME", "CARGO_TARGET_DIR", "RUSTUP_HOME", "XDG_CACHE_HOME")},
    }) + "\n")
if os.environ.get("TEST_CARGO_FAIL_MANIFEST") in sys.argv[1:]:
    sys.exit(23)
'''


def package_manifests():
    return [line for line in INVENTORY.read_text().splitlines()
            if line and not line.startswith("#")]


class PackageInventoryTests(unittest.TestCase):
    def test_inventory_matches_owned_manifests_without_duplicates_or_vendor(self):
        expected = {"Cargo.toml", "workgraph/Cargo.toml"}
        expected.update(str(path.relative_to(REPO_ROOT))
                        for path in (REPO_ROOT / "crates").rglob("Cargo.toml")
                        if "target" not in path.relative_to(REPO_ROOT).parts)
        manifests = package_manifests()
        self.assertCountEqual(manifests, expected)
        for manifest in manifests:
            self.assertTrue((REPO_ROOT / manifest).is_file(), manifest)


class TestRunnerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="farcaster-test-runner-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        tools = self.root / "tools"
        tools.mkdir()
        cargo = tools / "cargo"
        cargo.write_text(f"#!{sys.executable}\n" + MOCK_CARGO)
        cargo.chmod(0o755)
        self.log = self.root / "cargo.jsonl"
        self.caller_data = self.root / "caller-data"
        self.caller_data.mkdir()
        (self.caller_data / "keep").write_text("caller data")
        self.environment = dict(os.environ,
                                PATH=str(tools) + os.pathsep + os.environ["PATH"],
                                TMPDIR=str(self.root), TEST_CARGO_LOG=str(self.log),
                                FARCASTER_DATA_DIR=str(self.caller_data))
        self.environment.pop("TEST_CARGO_FAIL_MANIFEST", None)

    def run_script(self, *args, cwd=REPO_ROOT, stdin=""):
        result = subprocess.run(["sh", str(SCRIPT), *args], cwd=cwd,
                                env=self.environment, input=stdin,
                                capture_output=True, text=True)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(calls)
        data_paths = {call["data"] for call in calls}
        self.assertEqual(len(data_paths), 1)
        for data_path in data_paths:
            self.assertNotEqual(data_path, str(self.caller_data))
            self.assertFalse(Path(data_path).exists(), "test data was not cleaned up")
        self.assertEqual((self.caller_data / "keep").read_text(), "caller data")
        expected_env = {key: self.environment.get(key) for key in PRESERVED_ENV}
        for call in calls:
            self.assertEqual(call["environment"], expected_env)
        return result, calls

    def test_default_runs_every_package_once_from_repository_root(self):
        result, calls = self.run_script(cwd=self.root, stdin="caller input\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call["args"] for call in calls],
                         [["test", "--manifest-path", manifest]
                          for manifest in package_manifests()])
        self.assertEqual({call["cwd"] for call in calls}, {str(REPO_ROOT)})
        self.assertEqual(calls[0]["stdin"], "caller input\n")

    def test_focused_arguments_pass_through_once_in_callers_directory(self):
        args = ("--manifest-path", "relative package/Cargo.toml", "--offline",
                "a test filter", "--", "--exact", "--nocapture")
        result, calls = self.run_script(*args, cwd=self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["args"], ["test", *args])
        self.assertEqual(calls[0]["cwd"], str(self.root))

    def test_default_stops_at_failed_package_and_preserves_exit_status(self):
        manifests = package_manifests()
        self.environment["TEST_CARGO_FAIL_MANIFEST"] = manifests[1]
        result, calls = self.run_script()
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertEqual([call["args"] for call in calls],
                         [["test", "--manifest-path", manifest] for manifest in manifests[:2]])

    def test_focused_failure_preserves_exit_status(self):
        manifest = "relative package/Cargo.toml"
        self.environment["TEST_CARGO_FAIL_MANIFEST"] = manifest
        result, calls = self.run_script("--manifest-path", manifest)
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertEqual(len(calls), 1)


if __name__ == "__main__":
    unittest.main()
