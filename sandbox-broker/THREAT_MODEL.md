# Threat Model

## Trust

Trust these inputs:

- the user choice shown by Pi;
- global machine policy loaded outside the workspace;
- this broker binary and its fixed policy data at a Nix-store or other host-owned path;
- the Pi extension and its private broker pipes;
- the host OS sandbox and, when enabled, the host-owned network proxy.

Do not trust:

- model output;
- shell text, programs, interpreters, descendants, or their output;
- project files, including `.pi` in the workspace;
- project sandbox config as a source of added rights;
- saved rights until Pi validates their shape and real workspace key;
- paths or environment values sent in an `exec` request;
- macOS unified denial logs as a full audit trail.

The user account and Pi host process remain outside this boundary. The broker limits command children; it does not protect against a hostile Pi extension running in the host process.

## Current release status

The native backend is the default on macOS and Linux. The macOS unsandboxed release gate passed. Protocol v2 keeps IP network access blocked and adds at most 16 exact Unix socket paths from trusted machine config. It still has no background-job support. A session-long, bounded macOS denial collector now emits structured hints with `complete: false`. Process-group cleanup, bounded pipe draining, and a best-effort macOS descendant tracker have landed. The tracker registers the root before the launch barrier opens, follows kqueue fork events with `proc_listchildpids` snapshots, and checks process start times before signaling observed survivors.

Unified denial records carry a PID but no process start time. A fast PID reuse or delayed record can therefore misattribute a hint even though cleanup signaling still checks process identity. Hints always need user approval and never prove command membership.

A child can still win the non-atomic fork-and-enumeration race, then leave the process group with `setpgid`, `setsid`, or a double fork. Public unprivileged macOS APIs do not provide a kill-and-reap container for such children; creating a new kernel coalition fails with `EPERM` for a normal user process. Pi explicitly places deliberate daemon escape outside the native backend's threat model. Any survivor keeps its Seatbelt limits, but it may continue using CPU and rights that the command received until it exits or the user kills it.

The Rust broker now has the default Linux Bubblewrap backend with a fixed binary path, read-only root, ordered exact write and deny mounts, user/PID/network/IPC/UTS namespaces, private `/proc`, `no_new_privs`, a reviewed blocked-network seccomp filter, and PID-namespace teardown. Its ignored release gate still needs to pass on x86_64 and aarch64 Linux. Missing Bubblewrap or unavailable unprivileged namespaces fail readiness rather than falling back.

Bubblewrap can mask only concrete paths. Linux expands existing secret-name glob matches under the active workspace before launch, with strict scan bounds, then mounts those matches after writable roots. Fixed hard denies separately protect SSH, cloud, auth, and control paths in the broker HOME. Directory symlinks into the immutable, globally readable Nix store are scan boundaries; ordinary user-directory symlinks are followed. The host user and Pi process are trusted, so a host-created post-snapshot secret is outside this boundary. A sandboxed command can create a new matching name in a writable tree, but that file contains data the command already controls. Linux v1 does not claim dynamic path-pattern mediation.

## Security rules

1. **Fail closed.** Broker startup, protocol, policy, proxy, Seatbelt, bubblewrap, or child-start failure blocks the command. No request can select an unsandboxed mode.
2. **Fresh rights.** Each command gets a new OS sandbox. One-time rights occur only in that command's immutable request and command ID.
3. **Two checks.** TypeScript resolves paths for UI. Rust resolves them again against the request cwd, canonicalizes existing ancestors, rejects relative paths, and applies hard denies last.
4. **Protected control state.** Commands cannot write the broker binary, broker policy, global `~/.pi`, global `~/.codex`, or auth and secret roots. Base writes keep existing `.git` and `.pi` paths below the active workspace read-only; only an exact approved command grant may add those roots. Linux also masks a missing active-workspace `.git` or `.pi` for the command and removes the empty mount target afterward. A new nested `.git` or `.pi` name created after Linux's snapshot contains command-created data and is not dynamic path mediation; Pi never loads a nested project `.pi`. The trusted host creates missing fixed development-cache directories before launch; their rights exclude package-manager config, credential files, and global install bins. Cache rights that overlap the active workspace are omitted, while cache Git data outside the workspace does not become a project-control grant.
5. **No path alias escape.** Existing symlinks resolve before policy build. For a missing leaf, the broker resolves the nearest existing ancestor and appends checked normal components. Tests cover symlinks, `..`, missing paths, and protected children under broad roots.
6. **Private control channel.** Commands inherit only their stdin/stdout/stderr and needed job handles. They do not inherit broker protocol handles or a public control socket.
7. **Lifecycle control.** On macOS, the backend registers the root before the launch barrier and combines process-group cleanup with best-effort descendant observations. It does not claim atomic ownership of a child that deliberately wins the macOS fork-and-reparent race. Linux uses Bubblewrap's PID namespace and init/reaper as the descendant boundary; its release gate must prove that cancellation, timeout, shutdown, `setsid`, and double-fork cases leave that namespace empty.
8. **Bounded data.** Frame, request, output, diagnostic, active-command, process-observation, denial, and later job limits are fixed. The broker drains capped output and marks it truncated. The macOS tracker keeps at most 4,096 process identities per command; the collector also caps raw lines, retained records, and per-command results.
9. **Explicit local service rights.** IP network access stays blocked. Unix socket paths come only from trusted machine config, stay separate from file rights, use exact Seatbelt filters, and never include the background-job control socket. A later approved-host stage must use a host-owned allowlisting proxy.
10. **Hints do not grant.** The Seatbelt denial collector may explain exact rights. The extension checks every path against base rights, saved or command rights, hard protected paths, and configured denies, then asks the user before retrying. Four distinct sibling-file hints may produce one explicit choice between those exact files and their recursive parent folder; the parent is never added without that choice. Broader approved trees retain hard-denied subtrees as carve-outs. Missing, late, unrelated, ambiguous, or `/dev` device denial data never adds access.
11. **Environment is replaced.** The child receives the filtered map in its request. It does not inherit the broker environment. The broker adds only fixed status markers and later required proxy values.
12. **Background parity.** Before native background jobs ship, they must use the same policy builder. Their one-time rights must last for that job's sandbox only and never enter another job.

## Main attacks and checks

| Attack | Required control |
| --- | --- |
| Change policy, broker, or approval records | Hard read/write policy; broker path and global control roots denied |
| Consume a sibling's one-time right | No shared grant queue; rights carried on one command ID |
| Escape through symlink or `..` | Double normalization; nearest-existing-parent resolution; protected carve-outs |
| Forge broker output | Framed private pipe; base64 child chunks; protocol handles closed in child |
| Leave an ordinary descendant after timeout | Process-group cleanup plus start-time-checked signaling of tracker observations |
| Deliberately win the fork/reparent tracking race | Out of scope on native macOS; the survivor remains under its command's Seatbelt profile |
| Reach Docker, SSH agent, tmux, or another local service | Unix sockets denied unless listed; reserved job socket always denied to normal bash |
| Exfiltrate through network | Network blocked or forced through host allowlist proxy; no broad local targets |
| Obtain a broad grant from an app's vague error | Explicit preflight rights or four exact safe sibling hints plus a clear recursive-folder choice; no prose guessing or automatic widening |
| Redirect an implicit cache root with a symlink | Omit fixed cache rights reached through symlinks; broker canonicalization remains authoritative |
| Poison a shared development cache for a later build | Residual risk; use separate users or disposable homes when workspaces do not trust each other |
| Exhaust broker memory or disk | Hard frame/output/denial/job limits; no unbounded log file |
| Trigger an unsandboxed fallback | Readiness gate and no protocol field for bypass |

## Out of scope

- A hostile host user, root process, kernel, or altered `/usr/bin/sandbox-exec`.
- Protecting the host from trusted Pi extensions, since extensions run in Pi's host process.
- Proving that macOS unified logging reports every denial.
- Interactive PTY support in the first normal-bash milestone.
- Guaranteed collection of a child that deliberately escapes its process group and the non-atomic macOS descendant tracker. Strict lifetime containment requires a stronger boundary such as a disposable VM or an entitled system service.

## Release gates

`tests/macos_release.rs` is the unsandboxed macOS gate. It passes with filesystem rules, blocked network and sockets, environment replacement, output limits, structured denial collection for a generic application error, cancellation, timeout, shutdown, process-group cleanup, and cleanup of an observed detached child. Deliberate fast `setsid` or double-fork escape is not a macOS release assertion.

Linux has an ignored release-gate scaffold that still must run and expand on
x86_64 and aarch64. It must cover read-only root mounts, exact writable mounts,
hidden read denies, protected child mounts, symlink and missing-path cases,
blocked network and host Unix sockets, user/PID namespace availability,
`no_new_privs`, seccomp, environment, framing, output bounds, cancellation,
timeout, shutdown, and strict descendant cleanup. Native is the default, but
that matrix must pass before declaring the Linux backend production-ready.
