#!/usr/bin/env python3
"""Bound one live test command and clean up only its process group."""

import argparse
import os
import signal
import subprocess
import sys


def stop_group(process, grace):
    # start_new_session makes this PID a group owned solely by this test.
    group = process.pid
    if group == os.getpgrp():
        raise RuntimeError("Refusing to terminate the runner's process group")
    try:
        os.killpg(group, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(group, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Some macOS restrictions also deny signalling an already-gone group.
        # Do not call that proof that all descendants exited.
        print("E2E_CLEANUP_UNCONFIRMED: cannot signal the remaining test group", file=sys.stderr)
    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--grace", type=float, default=5)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command or args.timeout <= 0 or args.grace < 0:
        parser.error("a command, positive timeout, and nonnegative grace are required")

    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, start_new_session=True)

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        result = process.wait(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        print(f"E2E_TIMEOUT: command exceeded {args.timeout:g}s", file=sys.stderr, flush=True)
        stop_group(process, args.grace)
        return 124
    except KeyboardInterrupt:
        stop_group(process, args.grace)
        return 130
    # Normal-case cleanup belongs to the Rust fixture, which owns and closes
    # its real harness. The watchdog handles only timeouts and interruptions.
    return result if result >= 0 else 128 - result


if __name__ == "__main__":
    try:
        sys.exit(main())
    except OSError as error:
        print(f"E2E_CLEANUP_ERROR: {error}", file=sys.stderr, flush=True)
        sys.exit(1)
