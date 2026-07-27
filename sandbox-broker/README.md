# Pi Sandbox Broker

This directory holds Pi's OS sandbox broker. It defines the private protocol, threat model, source record, and the first macOS Seatbelt backend. The backend supports one foreground command, command-bound file and tree rights, hard denies, blocked network, a launch barrier, bounded output, timeout, cancellation, shutdown cleanup, and best-effort structured Seatbelt denial hints.

The sandbox extension can use this broker through the opt-in global `backend: "native-preview"` setting on macOS. Linux reports `can_exec: false`, and macOS reports false if `/usr/bin/sandbox-exec`, the hard host policy, or the fixed `/usr/bin/log` denial collector is unavailable. Codex remains the default backend.

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

See [PROTOCOL.md](PROTOCOL.md), [THREAT_MODEL.md](THREAT_MODEL.md), [UPSTREAM.md](UPSTREAM.md), and [LINUX_BACKEND.md](LINUX_BACKEND.md).

## Linux work remaining

Linux currently reports `can_exec: false`; the client accepts only macOS/Seatbelt, and machine config keeps Linux on Codex. The Linux milestone must resolve dynamic secret-name glob enforcement, add a pinned bubblewrap launcher, read-only root and protected mount planning, user/mount/PID/network namespaces, `no_new_privs`, seccomp, strict process cleanup, Linux readiness/client wiring, Nix packages for x86_64 and aarch64, and a full Linux release gate. Protocol v1 must keep network, host Unix sockets, background jobs, and emitted denial hints unavailable. The full implementation and test checklist is in [LINUX_BACKEND.md](LINUX_BACKEND.md).

## Current checks

```sh
cargo test --manifest-path sandbox-broker/Cargo.toml
cargo clippy --manifest-path sandbox-broker/Cargo.toml --all-targets -- -D warnings
# Release gate: run outside any existing Seatbelt profile. This covers the
# full broker flow and cleanup of an observed detached child.
cargo test --manifest-path sandbox-broker/Cargo.toml -- --ignored
```

The macOS integration and extension-client gates pass. The release gate checks that a command which hides `EPERM` behind a generic app error still yields an exact structured denial hint. Cleanup combines a process group with a best-effort kqueue descendant tracker and process start-time checks. Deliberate fast `setpgid`, `setsid`, or double-fork escape is out of scope because macOS offers no unprivileged atomic owner for that process tree. A survivor remains under its command's Seatbelt profile. Protocol v1 keeps network and Unix sockets blocked and has no native background-job support.
