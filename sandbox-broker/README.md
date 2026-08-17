# Pi Sandbox Broker

This is the native shell runner used by Pi's sandbox extension. One broker
runs for each Pi session and launches every foreground command in a fresh OS
sandbox. It communicates with the extension over private inherited pipes and
never falls back to a plain host process.

The extension owns machine configuration, the checked-in project policy, and
explicit `request_access` interaction. The Rust broker validates paths again,
applies hard denies, builds the platform policy, and owns command cleanup.

## Backends

- macOS uses Seatbelt and returns best-effort structured denial hints. A
  broker-owned helper reports protected read probes as missing to compatible
  tools while Seatbelt keeps the deny in force. Cleanup combines a process
  group with a bounded descendant tracker.
- Linux uses a fixed Bubblewrap binary, a read-only host root, user and PID
  namespaces, a private `/proc`, `NoNewPrivs`, and a private network namespace.
  PID namespaces provide the command lifetime boundary.

Both backends support one foreground command, immutable file and tree rights,
hard denies, filtered environments, bounded output, timeouts, cancellation,
interactive stdin, and shutdown cleanup. Protocol v4 can apply active project
policy for local servers, route through one host-owned proxy, or do both.
macOS adds loopback rules and permits Unix socket bind only below effective write
roots; connecting to an existing Unix socket still needs an exact machine
right. Linux keeps local ports in a private network namespace and blocks the
user command from opening host Unix sockets. macOS may also receive a small set
of trusted exact Unix socket paths; Linux rejects those general paths.

The extension calls this sole and default backend `native-preview`. It starts a
separate broker for each background job so the policy snapshot captured at job
start stays immutable. The trusted extension passes the canonical managed-cache
root at broker startup only when it has a non-overlapping base write right. The
broker validates that root beneath `HOME` and exempts only environment-file
name denies inside it.

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
# Run the gate matching the unsandboxed host platform.
cargo test --manifest-path sandbox-broker/Cargo.toml --test macos_release -- --ignored --test-threads=1
cargo test --manifest-path sandbox-broker/Cargo.toml --test linux_release -- --ignored --test-threads=1
```

The extension also has a real-broker gate for one-run denial diagnostics,
active file and tree grants, Bun optional-file handling, the macOS Seatbelt
backstop, local test ports, the host allowlist proxy, bypass denial, and
background jobs:

```sh
cargo build --manifest-path sandbox-broker/Cargo.toml
npm run check:e2e --prefix extensions/sandbox
```

The macOS gate and extension end-to-end gate pass. The Linux gate automates
filesystem, namespace, seccomp, proxy bridge, direct Unix-socket denial,
environment, framing, output, cancellation, timeout, shutdown, and detached
descendant checks. The new Linux proxy case has not yet run on a Linux host.
