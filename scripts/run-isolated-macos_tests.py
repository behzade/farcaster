#!/usr/bin/env python3
"""Test launcher reuse without sending any events to macOS apps."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("run-isolated-macos.sh")


class IsolatedLauncherTests(unittest.TestCase):
    def test_running_instance_needs_no_open_or_executable_change(self):
        with tempfile.TemporaryDirectory(prefix="farcaster-launcher-test-") as directory:
            root = Path(directory)
            bundle = root / "Farcaster Isolated.app"
            executable = bundle / "Contents/MacOS/farcaster"
            executable.parent.mkdir(parents=True)
            executable.write_text("running build")
            commands = root / "bin"
            commands.mkdir()
            fixtures = {
                "uname": "printf 'Darwin\\n'",
                "cat": 'printf \'%s\\n\' "$TEST_ISOLATED_BUNDLE"',
                "ps": 'printf \'%s/Contents/MacOS/farcaster\\n\' "$TEST_ISOLATED_BUNDLE"',
            }
            for name in ("open", "cp", "codesign", "mktemp"):
                fixtures[name] = f'printf "{name}\\n" >> "$TEST_LAUNCH_LOG"; exit 91'
            for name, body in fixtures.items():
                command = commands / name
                command.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
                command.chmod(0o755)
            log = root / "launch.log"
            result = subprocess.run(
                ["/bin/sh", str(SCRIPT)],
                env=dict(os.environ, PATH=f"{commands}:/usr/bin:/bin",
                         TEST_ISOLATED_BUNDLE=str(bundle), TEST_LAUNCH_LOG=str(log)),
                capture_output=True, text=True, check=True,
            )
            self.assertEqual(result.stdout.strip(), str(bundle))
            self.assertFalse(log.exists(), "reuse must not reopen, copy, sign, or create a bundle")
            self.assertEqual(executable.read_text(), "running build")


if __name__ == "__main__":
    unittest.main()
