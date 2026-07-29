# Linux Bubblewrap Backend Work

## Current status

The Rust broker now has a foreground Bubblewrap backend and reports `linux`/`bubblewrap` only after a real namespace, `/proc`, seccomp, and `NoNewPrivs` self-test. It uses the same protocol-v1 validation, output, timeout, cancellation, and shutdown path as macOS, while Linux PID namespaces own descendant teardown. The client accepts the fixed Linux pair and Nix injects Bubblewrap's exact store path into the broker. Native is now the default by user decision, although no Linux release host has run the ignored integration gate yet.

The first Linux release should match the current native protocol v2 execution scope, while leaving macOS-only denial collection and socket paths out:

- one foreground command at a time;
- read, write, exact-file, exact-tree, and hard-deny policy;
- blocked network and host Unix sockets;
- filtered environment, bounded output, timeout, cancellation, and shutdown;
- no native background jobs, approved network hosts, Unix socket grants, PTY, or Linux denial hints.

Do not widen Linux policy while adding the v2 transport. The existing `denials` event is active on macOS and remains optional for Linux because Linux enforcement does not use unified Seatbelt logs. Add the other features as separate work after the blocked-network backend ships.

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
- reapply active-workspace `.git`, project `.pi`, global control roots, package-manager control files, secrets, and explicit denies after broad mounts while leaving configured development-cache Git data writable;
- keep denied reads hidden, not merely read-only;
- preserve file-versus-directory type;
- handle missing approved files and trees without broadening the parent;
- resolve existing symlinks and nearest existing parents before building mounts;
- reject aliases that conflict with a hard deny;
- create private `/proc`, `/dev`, and temporary mount points as required by the launcher;
- expose only runtime files needed for normal command execution.

Generate a mount plan first, validate it, then turn it into bubblewrap arguments. Do not let argument order silently undo protected child mounts.

#### Glob deny snapshot semantics

Bubblewrap protects concrete mount targets, not future names. Pi follows the reviewed Codex startup-snapshot approach for protocol v2: it expands existing matches with strict traversal, depth, and match caps, then masks those paths after writable mounts. Root-wide patterns scan the active workspace rather than the whole host root; fixed hard denies separately protect SSH, cloud, auth, and control paths in the broker HOME. More specific patterns scan their fixed non-glob prefix. Ordinary directory symlinks are followed so they cannot bypass a deny, while directory symlinks into the immutable, globally readable Nix store are scan boundaries. Scan errors or cap overflow reject the command.

The host user and Pi process are trusted by the threat model, so a trusted host process creating a new secret after the snapshot is not an attacker in this boundary. A sandboxed command may create a new matching name in a writable tree; that file contains data the command already controls and is not a pre-existing host secret. This is intentionally name-snapshot protection, not a claim that Bubblewrap implements dynamic path-pattern mediation. Tests must cover existing matches, scan bounds, and this documented limit.

#### Denial feedback

Linux Bubblewrap does not report the denied pathname to the broker. Declared rights are therefore the reliable pre-launch path. As a limited fallback, the extension may extract one exact absolute path and directory intent from a recognized access-error line, apply the normal protected-path and active-policy checks, ask the user, and retry within the same tool call. Application output cannot grant access by itself.

### 4. Add namespaces and kernel controls

The launcher must preserve the controls used by the reviewed Codex path:

- user and mount namespace isolation;
- a PID namespace with an init/reaper so killing its owner empties the namespace;
- a blocked network namespace for protocol v2;
- IPC and other namespace isolation required by the selected source;
- `no_new_privs` before user code;
- the reviewed seccomp policy for the supported CPU architectures;
- no host Unix sockets unless a later protocol version grants and mounts one.

Treat PID namespace teardown as the Linux lifetime boundary. Test that descendants cannot survive cancellation, timeout, broker shutdown, or a double fork.

### 5. Wire readiness and the extension (implemented)

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

Native is now the default by user decision. Keep the Linux release gate as a
required production-readiness check rather than using it to select the default.

### 6. Package both Linux architectures (wired; builds pending)

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
- active-workspace `.git` and project `.pi` stay read-only until explicitly approved, while configured development-cache Git data stays writable;
- global `~/.pi`, `~/.codex`, auth files, SSH/AWS/GnuPG roots, and active-workspace `.env` and key files stay protected;
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
- approved network hosts, Unix socket grants, and background jobs remain unavailable;
- the first Linux broker emits no denial hints and rejects non-empty Unix socket paths, while the client keeps the active protocol v2 shape used by macOS.

## Definition of done

The Linux backend is ready only when all of these are true:

- the broker reports `can_exec: true` only after a real bubblewrap self-test;
- the extension accepts `linux`/`bubblewrap` and routes native foreground bash through it;
- filesystem, namespace, seccomp, output, cancellation, timeout, and shutdown tests pass on x86_64 and aarch64 Linux;
- Nix packages pin both the broker and bubblewrap paths;
- unavailable native Linux blocks rather than falling back;
- `UPSTREAM.md`, licenses, README, threat model, and machine config match the shipped behavior;
- machine config switches Linux from `codex` to `native-preview` only after the release gate passes.

Approved network hosts and Unix socket grants require a later protocol version. Native background jobs and any future Linux denial source remain separate reviewed milestones with their own tests.
