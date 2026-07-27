# Pi Native Sandbox Broker Handoff

## Implementation Update

Work continued from this handoff in the Pi checkout. The current tree now includes:

- `sandbox-broker/`: protocol v1, threat model, Apache provenance, a framed Rust broker, and a macOS Seatbelt preview backend;
- `extensions/sandbox/broker-client.ts` and `broker-policy.ts`: strict client framing and TypeScript-to-Rust policy mapping;
- an opt-in global `backend: "native-preview"` path on macOS, while `codex` remains the default backend;
- command-bound read, write, and exact network-host declarations for bash and Codex-backed background starts;
- no shared one-time network grant queue;
- pinned Nix builds for the broker and extension.

The preview keeps network and Unix sockets blocked and rejects native background starts. It has no denial collector or
Linux backend yet. Cleanup now combines a process group with a best-effort kqueue descendant tracker and process
start-time checks. Pi explicitly places deliberate fast `setpgid`, `setsid`, and double-fork escape out of scope because
macOS has no unprivileged atomic owner for that tree; survivors retain Seatbelt limits. The unsandboxed
`sandbox-broker/tests/macos_release.rs` gate passes, so native activation is available on macOS. Linux still uses Codex:
the Rust broker reports `can_exec: false`, the client accepts only macOS/Seatbelt, and no bubblewrap launcher or Linux
release gate exists. The authoritative remaining-work checklist is
[`sandbox-broker/LINUX_BACKEND.md`](sandbox-broker/LINUX_BACKEND.md). Do not treat the later stage text below as current
implementation status without checking the tree.

## Purpose

This handoff records the investigation into Pi's shell sandbox approval failures and the proposed move away from wrapping
`codex sandbox` for each shell call. The next session should start in this repository and design a Pi-owned Rust sandbox
broker using the relevant Apache-2.0 Codex Seatbelt and bubblewrap code.

This file is a design handoff, not an implementation plan that has already been approved in every detail. Confirm the
architecture with the user before making a large change.

## Current Snapshot

- Date: 2026-07-27
- Pi repository: `/Users/behzad/Projects/personal/pi`
- Pi commit: `2883b4bd518cd295e8d58301d22ea0d5f9f3d2df`
- Pi checkout: detached `HEAD`
- Codex repository: `/Users/behzad/Projects/personal/codex`
- Codex commit: `65ae4c26e088913176a50d6daeb742d00942caee`
- Current Pi sandbox: TypeScript extension under `extensions/sandbox`
- Current backend: one new `codex sandbox` process per `bash` call
- No code was changed during the investigation before this handoff was written.
- Do not commit without the user's clear approval.

Check both repositories before editing because either checkout may have changed after this handoff.

## User Goal

Pi should own a reliable, cross-platform shell sandbox instead of depending on the Codex CLI's debug behavior and text
output. The intended direction is to extract the relevant Seatbelt and bubblewrap parts from Codex into this repository,
keep their Apache-2.0 attribution, and expose a narrow structured interface to the Pi extension.

The goal is not to remove sandboxing or grant broad host access. The goal is to:

- keep commands constrained by the OS;
- let the user approve exact extra rights;
- bind one-time rights to one command;
- avoid guessing access needs from child stderr;
- avoid adding a fixed delay to every shell call;
- fail closed when the broker cannot establish the requested sandbox;
- keep background jobs constrained by the same policy.

## Triggering Failure

The Jabama repository says the `issues` CLI needs elevated access. Pi ran:

```sh
issues search view=issue number=79
```

The CLI returned:

```text
_tag: IssuesAgentInternalError
message: the Issues service is unavailable
```

The real cause was a sandbox write denial. The Issues CLI hid the underlying OS error, so Pi did not ask the user for
permission. Its mutable state is under:

```text
~/.local/share/issues
```

The directory includes SQLite files and automation lock files. A generic command may need similar external state and may
also hide `EPERM` or `EACCES`.

## Why The Current Pi Sandbox Missed It

The current retry path is in:

- `extensions/sandbox/index.ts`
- `extensions/sandbox/sandbox-failures.ts`

`createApprovingSandboxOps()` collects command output after a non-zero exit and calls
`parseFilesystemFailurePaths()`. That parser only accepts lines containing one of:

- `operation not permitted`
- `permission denied`
- `read-only file system`
- `failed to write file`

The same line must also contain one exact absolute path. If no safe path is found, the command failure is returned without
an approval prompt. That is safe but weak: any CLI that translates an OS error into a generic application error defeats
automatic permission discovery.

Pi intentionally exposes `request_network_permission` but no general filesystem permission tool. Direct `read`, `write`,
`edit`, and `ls` calls can be checked before execution because their target path is structured. Arbitrary shell commands
cannot be checked that way unless the shell request declares rights or the OS backend reports denials in a structured,
reliable form.

## Relevant Pi History

The current design has already tried and removed Codex denial logs.

### Switch To Codex IO Sandbox

```text
7e72026 feat: switch Pi to Codex IO sandbox
2ecac0c feat: harden sandboxing rules
```

These commits replaced the former Guardian path with the current Codex permission-profile wrapper.

### Enable `--log-denials`

```text
3855759 feat: better approval
64533eb fix: permission handling
```

Commit `3855759` added `--log-denials`, parsed Seatbelt denial summaries, suppressed those summaries from model-visible
output, prompted for grantable paths, and retried. Commit `64533eb` fixed the parser to accept the actual Codex summary
shape:

```text
=== Sandbox denials ===
(process) file-write-create /absolute/path
```

### Remove `--log-denials`

```text
a21d9ba
0b77850 fix: UI polish
```

These commits removed `--log-denials` and `sandbox-denials.ts`, replacing them with the current child-output parser. The
denial logger had races and could miss the event entirely. A proposed workaround was to add about 200 ms of delay to all
shell calls. Do not restore that workaround.

Useful history commands:

```sh
git show 3855759 -- extensions/sandbox
git show 64533eb -- extensions/sandbox
git show a21d9ba -- extensions/sandbox README.md
git log --all --oneline -- extensions/sandbox/sandbox-denials.ts
```

## What Codex `--log-denials` Actually Does

The feature originated in Codex commit:

```text
0271c20d8f add codex debug seatbelt --log-denials (#4098)
```

The commit message calls it a debugging tool and explicitly says it is best effort:

- `log stream` may drop logs;
- `kqueue` plus `proc_listchildpids` is not atomic;
- very short-lived descendants can be missed.

Current source at the inspected Codex checkout:

- `codex-rs/cli/src/debug_sandbox.rs`
- `codex-rs/cli/src/debug_sandbox/seatbelt.rs`
- `codex-rs/cli/src/debug_sandbox/pid_tracker.rs`

The current sequence is:

1. spawn `log stream`;
2. spawn `sandbox-exec` and the requested command without waiting for collector readiness;
3. start tracking the root process and descendants;
4. wait for the command to exit;
5. stop PID tracking;
6. immediately kill `log stream`;
7. parse collected JSON logs and keep records whose PIDs were seen.

This has at least three races:

1. **Collector startup:** the command may fail before `log stream` has subscribed.
2. **Collector shutdown:** the command may exit before the denial record reaches the stream, then Codex kills the stream.
3. **Descendant tracking:** a short-lived executable may fork, fail, and exit before `proc_listchildpids` observes it.

A long-running collector removes the first race and reduces the second. It does not make descendant discovery atomic and
does not turn unified logging into a guaranteed denial channel.

## Unmerged Codex Work Worth Studying

The inspected Codex clone contains two remote branches with a more careful denial collector:

```text
origin/codex/extract-seatbelt-denial-collector
  484518f28433c37d3142c49d7060bd35462ce352
  refactor(sandbox): extract Seatbelt denial collector

origin/dh--log-denials-in-exec
  f847460584b7f4ee472e6b30700a0754e915ecbf
  feat(sandbox): append Seatbelt denials to unified exec
```

Inspect them with:

```sh
git show 484518f284
git show f847460584
git show 484518f284:codex-rs/sandboxing/src/seatbelt_denials/mod.rs
git diff 16c7c79540c..origin/dh--log-denials-in-exec
```

The extracted collector adds:

- an event-based readiness wait, with a two-second timeout;
- a 100 ms post-exit log flush grace period;
- live publication of tracked PIDs to the reader;
- bounded denial storage for normal command execution;
- tests for parsing and bounded retention.

This is better than a 200 ms delay on every shell call. It still labels the collector best effort and still uses
non-atomic descendant tracking. Use it as source and prior work, not as proof that denial collection is complete.

## License And Provenance

Codex uses Apache License 2.0:

- `/Users/behzad/Projects/personal/codex/LICENSE`
- `/Users/behzad/Projects/personal/codex/NOTICE`

Apache 2.0 permits copying and modifying the relevant code. The extracted work must:

- include the Apache 2.0 license;
- preserve applicable copyright notices;
- carry the Codex NOTICE text where required;
- mark material Pi changes;
- record exact upstream commit IDs and source paths;
- retain notices for any other copied third-party work;
- avoid implying that OpenAI supports the Pi fork.

Create an `UPSTREAM.md` or equivalent next to the Rust helper. Record the source commit for every imported group and keep
future sync work reviewable. Do not paste large source files without provenance.

## Proposed Architecture

### 1. Pi-Owned Rust Broker

Add a small Rust program owned by this repository. A possible location is:

```text
sandbox-broker/
  Cargo.toml
  src/
  third-party/
  LICENSE-APACHE
  NOTICE
  UPSTREAM.md
```

Choose the final path after checking how it should fit the Nix flake. Do not pull the whole Codex workspace into Pi.
Adapt the minimum code behind a Pi-owned policy and protocol.

The broker should run as a direct child of the Pi extension. It should remain outside the sandboxed command process tree
because it must create fresh sandboxes and manage them. Communicate through inherited pipes using framed JSON or another
strict format. Do not open a world-accessible socket.

### 2. One Broker Per Pi Session

Start one broker at `session_start` and stop it at `session_shutdown`. The broker may own one persistent macOS denial
collector for the session. It should send a readiness result before Pi enables shell execution.

A session broker removes repeated `log stream` startup and avoids a fixed wait before each command. It also gives command
runs stable IDs and one place to manage cancellation and process groups.

### 3. Fresh OS Sandbox Per Command

Do not run every command in one long-lived Seatbelt process. Seatbelt rights are fixed when the process enters the
sandbox and cannot be widened later. A long-lived sandbox would make a one-time grant available to later commands or
would need a restart whenever rights change.

Instead, the broker should launch each command in a fresh sandbox using the rights approved for that call:

- macOS: `/usr/bin/sandbox-exec` with a generated Seatbelt profile;
- Linux: bubblewrap helper with generated mounts and network rules.

A direct launch barrier should let the broker register the root PID and command ID before the child begins user code.
Use a new process group for each command so cancellation and timeouts terminate the full tree.

### 4. No Tmux For Normal Bash

Tmux is not required for session ownership, command framing, output streaming, or cancellation. Pipes and process groups
are enough. Tmux should remain limited to the existing `background_job` use case unless a separate interactive-terminal
requirement appears.

Normal shell calls and background jobs must use the same policy builder. Background jobs may have a longer-lived child
sandbox, but their rights must still be fixed at start and scoped to that job.

### 5. Structured Broker Protocol

A first protocol should cover:

```text
ready
exec
stdout
stderr
exit
denials
cancel
shutdown
```

Each message should include a command ID. An `exec` request should include at least:

- command and argument form;
- working directory;
- filtered environment;
- timeout;
- exact read roots;
- exact write roots;
- exact network hosts or proxy policy;
- approved Unix socket roots;
- protected and denied paths;
- output limits.

The broker should return a structured exit result and separately return any observed denials. Never require the TypeScript
extension to parse prose emitted by the command.

Do not trust paths merely because the model supplied them. Normalize them in the extension and again in the broker.
Apply protected-path and deny policy after normalization.

### 6. Explicit Rights Remain The Reliable Path

Even with a persistent collector, macOS unified denial logs remain best effort. Treat denial records as permission hints,
not as proof that no other denial occurred.

The reliable flow should let a shell call declare narrow rights before launch. One possible Pi tool shape is:

```ts
{
  command: "issues search view=issue number=79",
  permissions: [
    {
      kind: "write",
      path: "~/.local/share/issues",
      reason: "Issues needs its database and lock files"
    }
  ]
}
```

The exact API is undecided. The important rules are:

- prompt before launch;
- bind `allow once` to the exact command ID;
- do not store one-time grants in a shared consumable queue;
- never allow a protected or explicit-deny path;
- save only `always allow in this workspace` decisions;
- make rights visible in the approval UI;
- do not infer broad parent paths from vague failures.

Observed Seatbelt denials can still support an automatic prompt-and-retry path when they contain one safe exact target.
Missing denial data must not weaken enforcement.

## macOS Source Scope

Start by studying and reducing these Codex sources:

```text
codex-rs/sandboxing/src/seatbelt.rs
codex-rs/sandboxing/src/seatbelt_base_policy.sbpl
codex-rs/sandboxing/src/seatbelt_network_policy.sbpl
codex-rs/sandboxing/src/restricted_read_only_platform_defaults.sbpl
codex-rs/sandboxing/src/seatbelt_tests.rs
```

For the improved collector, study the unmerged branch version:

```text
codex-rs/sandboxing/src/seatbelt_denials/mod.rs
codex-rs/sandboxing/src/seatbelt_denials/pid_tracker.rs
codex-rs/sandboxing/src/seatbelt_denials/seatbelt_denials_tests.rs
```

Avoid taking Codex network-proxy and protocol dependencies unless Pi needs their behavior. Define a narrow Pi policy type
and adapt the policy generation around it.

macOS invariants:

- use `/usr/bin/sandbox-exec`, never a PATH-resolved replacement;
- fail closed when profile generation or `sandbox-exec` fails;
- keep protected metadata read-only;
- keep secret paths unreadable;
- keep the broker and Pi policy files inaccessible to sandboxed commands;
- do not allow sandboxed children to reach broker control pipes they do not need;
- keep network and Unix socket rules explicit;
- retain the current shell environment filtering.

## Linux Source Scope

Codex's current Linux path is larger than its Seatbelt argument builder. Relevant sources include:

```text
codex-rs/sandboxing/src/landlock.rs
codex-rs/sandboxing/src/bwrap.rs
codex-rs/linux-sandbox/src/bwrap.rs
codex-rs/linux-sandbox/src/linux_run_main.rs
codex-rs/linux-sandbox/src/launcher.rs
codex-rs/linux-sandbox/src/bundled_bwrap.rs
codex-rs/linux-sandbox/src/exec_util.rs
codex-rs/linux-sandbox/src/proxy_routing.rs
```

The current Codex Linux helper:

- uses bubblewrap as the default filesystem sandbox;
- makes the filesystem read-only by default;
- bind-mounts writable roots;
- reapplies protected subpaths as read-only;
- handles missing protected targets and symlinks;
- isolates user and PID namespaces;
- optionally isolates the network namespace;
- applies seccomp and `no_new_privs`;
- supports system and bundled bubblewrap;
- handles signal forwarding and cleanup.

Do not reduce this to a few unreviewed `bwrap` flags. The current broker has no Linux execution path: `main.rs` returns
`can_exec: false`, `executor.rs` launches Seatbelt directly, the TypeScript client accepts only `macos`/`seatbelt`, and the
extension rejects `native-preview` outside macOS. Start with protocol v1 foreground parity and keep approved network,
Unix socket grants, background jobs, PTY, and denial hints out of that milestone. Preserve the security tests for every
imported behavior. The full implementation order, packaging work, release matrix, and definition of done are in
[`sandbox-broker/LINUX_BACKEND.md`](sandbox-broker/LINUX_BACKEND.md).

## Policy Ownership

Pi should own one platform-neutral permission model. The current TypeScript policy lives in:

- `extensions/sandbox/codex-command.ts`
- `extensions/sandbox/io-policy.ts`
- `extensions/sandbox/io-permissions.ts`

Do not keep two independent policy engines that can drift. Choose one clear boundary:

- TypeScript owns user config, approval state, and display;
- Rust owns final normalization, platform policy generation, and process execution;
- the Rust broker rejects any request that violates hard protected paths;
- the extension cannot request a `no sandbox` mode through the broker protocol;
- command execution stays blocked when the broker is unavailable.

The project `.pi/sandbox.json` may only tighten global policy. Preserve that rule.

## Threat Model

Assume the model and shell command are untrusted. The user approval is the authority for extra access.

The design must prevent:

- a command from changing the broker or its policy;
- a command from consuming another command's one-time grant;
- a parallel command from gaining a sibling's rights;
- a path alias or symlink from escaping approved roots;
- a relative path from being resolved against the wrong directory;
- a denied child from talking to arbitrary local services through Unix sockets;
- a child from inheriting secrets through environment variables;
- a timed-out command from leaving descendants alive;
- a protocol message from being forged by command stdout;
- output or denial logs from growing without a hard cap;
- a missing denial event from causing an automatic broad grant;
- a broker failure from falling back to unsandboxed execution.

Keep broker protocol stdout separate from child stdout, for example by using dedicated inherited file descriptors or a
framed channel that child processes never inherit.

## Suggested Delivery Stages

### Stage 1: Design And Provenance

- Write the broker protocol and threat model.
- Pick repository paths and Nix package boundaries.
- Add Apache attribution and `UPSTREAM.md`.
- List the exact Codex files and commits to import.
- Decide the explicit permission request shape.

### Stage 2: macOS Broker Without Denial Discovery

- Import the minimum Seatbelt policy builder.
- Launch one fresh sandbox per command.
- Support output, timeout, cancellation, environment filtering, read/write roots, protected paths, network, and sockets.
- Keep explicit rights and current saved approval behavior.
- Cut the TypeScript extension over to the broker on macOS.

This stage should already remove the Codex CLI dependency from normal macOS shell calls.

### Stage 3: Persistent macOS Denial Collector

- Adapt the improved unmerged collector.
- Wait for collector readiness once per session.
- bind records to command IDs as safely as available data permits;
- bound all retained records;
- return structured denial hints;
- support prompt and retry only for an exact, policy-safe path;
- test very short commands and delayed log delivery;
- document that absence of a record is inconclusive.

Do not block Stage 2 on perfect denial discovery.

### Stage 4: Linux Bubblewrap Backend

- Choose and record an exact Codex source commit before importing Linux code.
- Split platform launch code so request validation and protocol handling remain shared.
- Add a pinned bubblewrap launcher with read-only root mounts, exact writable mounts, protected child mounts, hidden read denies, and safe missing-path handling.
- Resolve recursive glob hard denies with a reviewed dynamic enforcement design; bubblewrap, Landlock, seccomp, and pre-launch match expansion do not enforce names created after launch.
- Add user, mount, PID, and blocked-network namespaces, `no_new_privs`, reviewed seccomp rules, and PID-namespace teardown.
- Add Linux readiness self-tests and accept only the `linux`/`bubblewrap` readiness pair in the client.
- Package the exact broker and bubblewrap paths for x86_64 and aarch64 Linux.
- Pass the Linux integration and strict descendant-cleanup gates before machine config selects native Linux.

See [`sandbox-broker/LINUX_BACKEND.md`](sandbox-broker/LINUX_BACKEND.md) for the complete checklist.

### Stage 5: Background Jobs And Cleanup

- Route background job starts through the broker policy.
- Keep job control scoped to broker-created jobs.
- Remove obsolete Codex command-building code and stderr permission parsing after the new paths cover all supported cases.
- Update README and machine deployment docs.

## Tests To Require

### Broker Protocol

- readiness success and failure;
- malformed and oversized messages;
- command ID isolation;
- stdout and stderr cannot forge broker events;
- cancellation kills the process group;
- timeout kills descendants;
- broker shutdown cleans all owned children;
- output and denial caps.

### Permission Semantics

- workspace write works by default;
- external write fails without a grant;
- exact one-time external write works for one command only;
- persistent workspace grant reloads correctly;
- parallel commands cannot share one-time grants;
- protected paths never prompt and never become writable;
- explicit deny overrides an approval request;
- project config cannot broaden global policy;
- symlink and relative-path cases stay within the approved root;
- `.git` asks for repository control as one unit;
- project `.pi` asks for the project control directory;
- global `~/.pi` and `~/.codex` remain protected.

### macOS

- generated Seatbelt profile snapshots;
- `/usr/bin/sandbox-exec` is used directly;
- collector readiness is event-based, not a fixed startup sleep;
- delayed denial delivery is collected within the broker policy;
- short-lived child denial tests run repeatedly to expose races;
- an empty denial set does not trigger a grant or unsandboxed retry;
- unsafe socket and network access remain blocked.

### Issues Reproduction

Test the real failure class with a small fixture CLI that catches `EPERM` and emits only a generic application error. Do
not rely on the live Issues database in automated tests. Cover both flows:

- no declared right: command stays denied even if no denial hint arrives;
- declared exact right: user approval lets the command write its fixture state.

A manual check may then use:

```sh
issues search view=issue number=79
```

with an approved write right for `~/.local/share/issues`.

### Linux

Linux has no release gate yet. At minimum it must cover:

- root filesystem read-only and denied reads hidden;
- exact writable roots and files;
- nested protected paths reapplied after broad mounts;
- missing, symlinked, rename, and mount-order cases cannot escape;
- user, mount, PID, IPC, and blocked-network namespace rules;
- `no_new_privs` and reviewed seccomp on x86_64 and aarch64;
- fixed host-owned `bwrap` selection, never workspace `PATH`;
- cancellation, timeout, shutdown, `setsid`, and double-fork cleanup;
- unavailable bubblewrap or user namespaces fail readiness closed;
- protocol parity for environment, output, IDs, framing, and terminal ordering.

The full test matrix is in [`sandbox-broker/LINUX_BACKEND.md`](sandbox-broker/LINUX_BACKEND.md).

## Build And Packaging Notes

Current Pi checks include:

```sh
npm run check --prefix extensions/sandbox
node --test tests/governance.test.ts
nix flake check
nix build .#sandbox
```

The broker and extension now have pinned Nix packages, and the macOS extension contains the exact broker store path. The
Linux stage must keep that property and also inject an exact bubblewrap store path into the broker. Update:

- `flake.nix` for x86_64 and aarch64 Linux checks;
- `nix/pi-sandbox-broker.nix` with the pinned bubblewrap input;
- `nix/pi-sandbox-extension.nix` without weakening the broker path substitution;
- `README.md`, `UPSTREAM.md`, and machine config after release gates pass.

The deployed extension must verify the expected broker/backend pair and fail closed if either helper is unavailable. Do
not search the workspace or user `PATH` for the broker or bubblewrap.

## Decisions Still Needed

The macOS architecture and explicit-right shape are implemented. Remaining choices are:

1. Which exact Codex commit should supply the Linux launcher, mount, namespace, and seccomp reference?
2. What trusted mechanism will enforce recursive secret-name globs for paths created after launch? Bubblewrap-only snapshot masking is insufficient.
3. Should non-Nix Linux require a fixed system bubblewrap path, or should Pi package a vetted binary there too?
4. Which reviewed seccomp policy and architecture set should the first Linux release support?
5. After blocked-network Linux ships, should approved hosts use the same future host-owned proxy design as macOS?
6. Should native background jobs land before or after Linux foreground parity?
7. Should denial hints remain a later macOS-only task, or wait for one cross-platform diagnostic design?

None of these choices may weaken protocol v1 or delay fail-closed Linux readiness checks.

## First Commands For The Linux Session

```sh
cd /Users/behzad/Projects/personal/pi
git status --short --branch
sed -n '1,260p' sandbox-broker/LINUX_BACKEND.md
sed -n '1,180p' sandbox-broker/THREAT_MODEL.md
sed -n '1,220p' sandbox-broker/UPSTREAM.md
sed -n '1,260p' sandbox-broker/src/main.rs
sed -n '1,620p' sandbox-broker/src/executor.rs
sed -n '230,290p' extensions/sandbox/broker-client.ts
sed -n '1070,1120p' extensions/sandbox/index.ts
```

Then inspect the complete Linux source and test surface at the Codex commit proposed for import. Record that commit and
all copied paths in `UPSTREAM.md` before implementation. Keep the first Linux milestone to foreground protocol v1 with
blocked network and host Unix sockets. Do not switch Linux machine config from Codex until the release matrix in
`LINUX_BACKEND.md` passes on x86_64 and aarch64.
