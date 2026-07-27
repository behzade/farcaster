# Linux Bubblewrap Backend Work

## Current status

The Rust broker does not execute commands on Linux. `sandbox-broker/src/main.rs` reports `can_exec: false`, the extension accepts only a macOS `seatbelt` readiness frame, and `extensions/sandbox/index.ts` rejects `native-preview` outside macOS. Linux therefore stays on the Codex backend.

The first Linux release should match the current native protocol v1 scope:

- one foreground command at a time;
- read, write, exact-file, exact-tree, and hard-deny policy;
- blocked network and host Unix sockets;
- filtered environment, bounded output, timeout, cancellation, and shutdown;
- no native background jobs, approved network hosts, Unix socket grants, PTY, or denial hints.

Do not widen protocol v1 while adding Linux. Add those features as separate work after the blocked-network backend ships.

## Source and license work

Use the Codex files listed in [UPSTREAM.md](UPSTREAM.md) and the Linux source list in [SANDBOX_BROKER_HANDOFF.md](../SANDBOX_BROKER_HANDOFF.md) as references. Before copying code:

1. choose and record one exact Codex commit;
2. inspect the full dependency and test surface at that commit;
3. copy only the launcher, mount, seccomp, and cleanup code Pi needs;
4. mark Pi changes in source headers;
5. update `UPSTREAM.md`, `NOTICE`, dependency licenses, and `Cargo.lock` in the same change.

Do not copy a few bubblewrap flags without the related path, namespace, signal, and security tests.

## Implementation work

### 1. Split platform code

The current executor builds Seatbelt arguments directly. Refactor without changing the protocol:

- keep request validation and hard path policy shared;
- move macOS launch details behind a platform backend;
- add a Linux backend that returns a prepared command and cleanup handle;
- keep `/usr/bin/sandbox-exec` and macOS PID tracking macOS-only;
- keep terminal event ordering, output caps, cancellation state, and the launch barrier shared where possible.

Likely files:

- `sandbox-broker/src/main.rs`
- `sandbox-broker/src/executor.rs`
- `sandbox-broker/src/seatbelt.rs`
- new `sandbox-broker/src/linux.rs` or `src/linux/`
- `sandbox-broker/src/validation.rs`

### 2. Pin and check bubblewrap

Use a host-owned absolute bubblewrap path. A workspace `PATH` lookup is not allowed.

- Nix builds should inject the exact bubblewrap store path.
- A non-Nix build may use a reviewed fixed system path such as `/usr/bin/bwrap`.
- Broker readiness must run a small self-test that proves the required user, mount, and PID namespaces work on that host.
- Missing bubblewrap, disabled unprivileged user namespaces, missing kernel features, or a failed self-test must produce `can_exec: false` and block commands.

Decide whether the first non-Nix release requires system bubblewrap or ships a vetted binary. Record that choice before implementation.

### 3. Build the mount policy

The Linux mount plan must enforce the same normalized rights as Seatbelt:

- expose the host root read-only;
- bind approved writable trees and files at their exact paths;
- reapply `.git`, project `.pi`, global control roots, secrets, and explicit denies after broad mounts;
- keep denied reads hidden, not merely read-only;
- preserve file-versus-directory type;
- handle missing approved files and trees without broadening the parent;
- resolve existing symlinks and nearest existing parents before building mounts;
- reject aliases that conflict with a hard deny;
- create private `/proc`, `/dev`, and temporary mount points as required by the launcher;
- expose only runtime files needed for normal command execution.

Generate a mount plan first, validate it, then turn it into bubblewrap arguments. Do not let argument order silently undo protected child mounts.

### 4. Add namespaces and kernel controls

The launcher must preserve the controls used by the reviewed Codex path:

- user and mount namespace isolation;
- a PID namespace with an init/reaper so killing its owner empties the namespace;
- a blocked network namespace for protocol v1;
- IPC and other namespace isolation required by the selected source;
- `no_new_privs` before user code;
- the reviewed seccomp policy for the supported CPU architectures;
- no host Unix sockets unless a later protocol version grants and mounts one.

Treat PID namespace teardown as the Linux lifetime boundary. Test that descendants cannot survive cancellation, timeout, broker shutdown, or a double fork.

### 5. Wire readiness and the extension

Linux readiness should use:

```json
{
  "type": "ready",
  "version": 1,
  "platform": "linux",
  "backend": "bubblewrap",
  "can_exec": true,
  "max_frame_bytes": 1048576
}
```

Update:

- `sandbox-broker/src/main.rs` to select Seatbelt on macOS and bubblewrap on Linux;
- `extensions/sandbox/broker-client.ts` to accept only the expected backend for the current platform;
- `extensions/sandbox/index.ts` to allow `native-preview` on supported Linux hosts and show a Linux label;
- client tests for accepted and rejected platform/backend pairs;
- fail-closed behavior so native selection never falls back to Codex or an unsandboxed process.

Keep Linux on `backend: "codex"` in machine config until the Linux release gate passes.

### 6. Package both Linux architectures

Update the Nix package so the Linux broker closure contains the pinned bubblewrap binary and any seccomp data it needs. Verify:

- `x86_64-linux` build;
- `aarch64-linux` build;
- the extension contains the exact broker store path;
- the broker contains or receives the exact bubblewrap store path;
- no runtime lookup uses the workspace or user `PATH`.

Do not change the working macOS package path or Seatbelt tests while adding Linux.

## Required tests

Add a Linux release test, such as `sandbox-broker/tests/linux_release.rs`, that runs on a host where unprivileged namespaces are enabled.

### Readiness and failure

- valid bubblewrap self-test reports `linux` and `bubblewrap` with `can_exec: true`;
- missing or non-executable bubblewrap fails closed;
- disabled user namespaces fail closed;
- malformed policy never starts user code;
- broker loss never triggers a plain host fallback.

### Filesystem

- host root is read-only;
- workspace writes work;
- external writes fail without a grant and work with one exact grant;
- exact-file rights do not widen to the parent;
- missing file and tree grants create only the approved target;
- `.git` and project `.pi` stay read-only until explicitly approved;
- global `~/.pi`, `~/.codex`, auth files, SSH/AWS/GnuPG roots, `.env`, and key files stay protected;
- explicit denies override broad reads, writes, and grants;
- symlink, `..`, rename, and mount-order cases cannot escape a right;
- denied reads do not reveal file contents.

### Process and namespace

- `/proc` shows only the sandbox PID namespace;
- network interfaces and routes cannot reach the host or internet;
- UDP, TCP, and host Unix socket access fail;
- `no_new_privs` is set;
- selected seccomp rules load on x86_64 and aarch64;
- cancellation and timeout kill foreground, forked, `setsid`, and double-fork descendants;
- broker shutdown kills the command and exits cleanly;
- repeated starts leave no mounts, helper processes, or namespace children.

### Protocol parity

- environment replacement, output limits, framing, IDs, and terminal ordering match macOS;
- stdout and stderr cannot forge broker events;
- one-time rights stay on one command ID;
- a second active command is rejected;
- approved network hosts, Unix socket grants, background jobs, and denial hints remain rejected in protocol v1.

## Definition of done

The Linux backend is ready only when all of these are true:

- the broker reports `can_exec: true` only after a real bubblewrap self-test;
- the extension accepts `linux`/`bubblewrap` and routes native foreground bash through it;
- filesystem, namespace, seccomp, output, cancellation, timeout, and shutdown tests pass on x86_64 and aarch64 Linux;
- Nix packages pin both the broker and bubblewrap paths;
- unavailable native Linux blocks rather than falling back;
- `UPSTREAM.md`, licenses, README, threat model, and machine config match the shipped behavior;
- machine config switches Linux from `codex` to `native-preview` only after the release gate passes.

Native background jobs, approved network hosts, Unix socket grants, and denial collection remain later milestones unless their protocol and tests land in the same reviewed change.
