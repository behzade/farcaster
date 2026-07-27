# Pi Sandbox Broker

This directory holds Pi's OS sandbox broker. It defines the private protocol, threat model, source record, and the first macOS Seatbelt backend. The backend supports one foreground command, command-bound file and tree rights, hard denies, blocked network, a launch barrier, bounded output, timeout, cancellation, and shutdown cleanup.

The sandbox extension has a client for this broker, but it blocks native activation. Linux reports `can_exec: false`, and macOS reports false if `/usr/bin/sandbox-exec` or the hard host policy is unavailable. This keeps the current Codex-backed shell path in place while the native backend passes its release gates.

## Approved direction

- One broker process per Pi session.
- Private inherited stdin/stdout pipes; no public socket.
- One fresh OS sandbox and process group per command.
- TypeScript owns config, approval state, and UI.
- Rust checks paths again, builds the final OS policy, runs commands, and rejects hard protected paths.
- Optional rights sit on the `bash` call and bind to that tool call's command ID.
- macOS lands first. Linux keeps the current fail-closed backend during a short, clear migration.
- macOS denial logs are hints. Missing logs grant nothing.
- Linux first uses a fixed system or Nix-store bubblewrap path, never workspace `PATH`.

See [PROTOCOL.md](PROTOCOL.md), [THREAT_MODEL.md](THREAT_MODEL.md), and [UPSTREAM.md](UPSTREAM.md).

## Current checks

```sh
cargo test --manifest-path sandbox-broker/Cargo.toml
cargo clippy --manifest-path sandbox-broker/Cargo.toml --all-targets -- -D warnings
# Release gates: run outside any existing Seatbelt profile. This includes the
# full broker flow and hostile setpgid/setsid/double-fork fixtures.
cargo test --manifest-path sandbox-broker/Cargo.toml -- --ignored
```

Do not switch the extension to this binary until the macOS integration, hostile-child cleanup, and extension-client gates pass. A process group cannot contain a hostile child that calls `setpgid` or `setsid`; the release needs a stronger owner boundary rather than best-effort PID polling. Protocol v1 keeps network blocked and has no Unix socket or background-job support.
