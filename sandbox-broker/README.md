# Pi Sandbox Broker

This is the native shell runner used by Pi's sandbox extension. One broker
runs for each Pi session and launches every foreground command in a fresh OS
sandbox. It communicates with the extension over private inherited pipes and
never falls back to a plain host process.

The extension owns configuration, saved approvals, and user interaction. The
Rust broker validates paths again, applies hard denies, builds the platform
policy, and owns command cleanup.

## Backends

- macOS uses Seatbelt and returns best-effort structured denial hints. Cleanup
  combines a process group with a bounded descendant tracker.
- Linux uses a fixed Bubblewrap binary, a read-only host root, user and PID
  namespaces, a private `/proc`, `NoNewPrivs`, and a blocked-network seccomp
  filter. PID namespaces provide the command lifetime boundary.

Both backends support one foreground command, command-scoped file and tree
rights, hard denies, filtered environments, bounded output, timeouts,
cancellation, and shutdown cleanup. Protocol v2 keeps IP networking blocked
and does not support native background jobs. macOS may receive a small set of
trusted exact Unix socket paths; Linux rejects them.

The default extension config calls this backend `native-preview`. Global config
may select the Codex CLI backend instead.

## Documentation

- [PROTOCOL.md](PROTOCOL.md) defines the framed protocol.
- [THREAT_MODEL.md](THREAT_MODEL.md) records trust boundaries, guarantees, and
  known limits.
- [UPSTREAM.md](UPSTREAM.md) records the imported Codex source and licenses.

## Checks

```sh
cargo test --manifest-path sandbox-broker/Cargo.toml
cargo clippy --manifest-path sandbox-broker/Cargo.toml --all-targets -- -D warnings
```

The ignored integration tests are host-level release gates and must run outside
an existing sandbox:

```sh
cargo test --manifest-path sandbox-broker/Cargo.toml -- --ignored
```

The macOS gate passes. The Linux gate still needs coverage on each supported
release architecture before this broker should be treated as a portable
general-purpose sandbox.
