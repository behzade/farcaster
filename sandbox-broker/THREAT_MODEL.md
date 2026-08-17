# Threat Model

## Trust

Trust these inputs:

- the user choice shown by Pi;
- global machine policy loaded outside the workspace;
- the strictly validated `.pi/extensions/sandbox/sandbox.json` policy of a project the user marked trusted;
- this broker binary and its fixed policy data at a Nix-store or other host-owned path;
- the Pi extension and its private broker pipes;
- the host OS sandbox and, when enabled, the host-owned network proxy.

Do not trust:

- model output;
- shell text, programs, interpreters, descendants, or their output;
- project files generally, including executable project tools, before the user marks the project trusted;
- hot edits to project sandbox policy during a session (only an approved `request_access` update activates immediately);
- project policy rights until Pi strictly validates their version, portable shape, hard-deny precedence, and control roots;
- paths or environment values sent in an `exec` request;
- macOS unified denial logs as a full audit trail.

The user account and Pi host process remain outside this boundary. The broker limits command children; it does not protect against a hostile Pi extension running in the host process.

## Current release status

The native backend is the sole default on macOS and Linux. The macOS
unsandboxed release gate and the extension's real-broker end-to-end gate pass.
Protocol v4 keeps network blocked unless the active, previously approved
project policy grants local loopback, an exact host set through a command-scoped
proxy, or both. Protocol v3 added interactive stdin so the
extension can run each background job in a separate broker with its own rights.
macOS accepts at most 16 exact Unix socket paths from trusted machine config. A
session-long, bounded macOS denial collector emits structured hints with
`complete: false`. Process-group cleanup, bounded pipe draining, and a
best-effort macOS descendant tracker have landed. The tracker registers the root
before the launch barrier opens, follows kqueue fork events with
`proc_listchildpids` snapshots, and checks process start times before signaling
observed survivors. On macOS, a broker-owned dynamic library maps protected read
probes to `ENOENT` for tools that treat an optional file's Seatbelt `EPERM` as a
fatal error. It receives the full read-deny rules and does not scan workspace
folders. Seatbelt still enforces every deny if a process drops or cannot load
the library.

Unified denial records carry a PID but no process start time. A fast PID reuse or delayed record can therefore misattribute a hint even though cleanup signaling still checks process identity. Hints are bounded diagnostics only: they never grant access and never prove command membership.

A child can still win the non-atomic fork-and-enumeration race, then leave the process group with `setpgid`, `setsid`, or a double fork. Public unprivileged macOS APIs do not provide a kill-and-reap container for such children; creating a new kernel coalition fails with `EPERM` for a normal user process. Pi explicitly places deliberate daemon escape outside the native backend's threat model. Any survivor keeps its Seatbelt limits, but it may continue using CPU and rights that the command received until it exits or the user kills it.

The Rust broker has a Linux Bubblewrap backend with a fixed binary path,
read-only root, ordered exact write and deny mounts, user/PID/network/IPC/UTS
namespaces, private `/proc`, `no_new_privs`, a reviewed blocked-network seccomp
filter, and PID-namespace teardown. An approved local test-port right enables
private loopback without a host bridge. Proxy mode adds one fixed bridge
listener inside the network namespace. A host-only launcher connects that
listener to the validated proxy socket, then starts the user command under a
filter that denies `AF_UNIX`. Its automated, ignored host release gate still needs to pass on
x86_64 and aarch64 Linux; the new proxy bridge has not yet run on Linux. Missing
Bubblewrap or unavailable unprivileged namespaces fail readiness rather than
falling back.

Bubblewrap can mask only concrete paths. Linux expands existing secret-name glob matches under the active workspace before launch, with strict scan bounds, then mounts those matches after writable roots. Fixed hard denies separately protect SSH, cloud, auth, and control paths in the broker HOME. Directory symlinks into the immutable, globally readable Nix store are scan boundaries; ordinary user-directory symlinks are followed. The host user and Pi process are trusted, so a host-created post-snapshot secret is outside this boundary. A sandboxed command can create a new matching name in a writable tree, but that file contains data the command already controls. Linux does not claim dynamic path-pattern mediation.

## Security rules

1. **Fail closed.** Broker startup, protocol, policy, proxy, Seatbelt, bubblewrap, or child-start failure blocks the command. No request can select an unsandboxed mode.
2. **Fresh rights.** Each command gets a new OS sandbox and an immutable snapshot of the active project policy. Policy updates affect later commands only; there are no one-time rights or automatic retries.
3. **Two checks.** TypeScript resolves paths for UI. Rust resolves them again against the request cwd, canonicalizes existing ancestors, rejects relative paths, and applies hard denies last.
4. **Protected control state.** Commands cannot write the broker binary, broker policy, macOS conceal helpers, global `~/.pi`, global `~/.codex`, project `.pi`, or auth and secret roots. Base writes keep existing `.git` and `.pi` paths below the active workspace read-only; a validated project-policy grant may add `.git`, but never project `.pi` or a symlinked control root. The trusted host-side `request_access` implementation alone may conditionally update `.pi/extensions/sandbox/sandbox.json` after approval, and refuses a concurrent/manual edit. Linux also masks a missing active-workspace `.git` or `.pi` for the command and removes the empty mount target afterward. A new nested `.git` or `.pi` name created after Linux's snapshot contains command-created data and is not dynamic path mediation; Pi never loads a nested project `.pi`. The trusted host creates missing fixed development-cache directories before launch; their rights exclude package-manager config, credential files, and global install bins. Cache rights and environment-file exemptions that overlap the active workspace are omitted, while cache Git data outside the workspace does not become a project-control grant. Public source files named `.env` or `.env.*` may be read and written only inside that validated managed cache; SSH, auth, key, PEM, and control denies remain in force there.
5. **No path alias escape.** Checked-in rights are project-relative or `~/` home-relative; absolute request paths are converted only beneath those roots and are otherwise rejected. Project rights crossing an existing symlink are rejected and revalidated immediately before each broker request, including a missing path later replaced by a symlink. The broker still resolves paths independently, checks nearest existing ancestors, and applies protected carve-outs.
6. **Private control channel.** Commands inherit only their stdin/stdout/stderr and needed job handles. They do not inherit broker protocol handles or a public control socket.
7. **Lifecycle control.** On macOS, the backend registers the root before the launch barrier and combines process-group cleanup with best-effort descendant observations. It does not claim atomic ownership of a child that deliberately wins the macOS fork-and-reparent race. Linux uses Bubblewrap's PID namespace and init/reaper as the descendant boundary; its release gate must prove that cancellation, timeout, shutdown, `setsid`, and double-fork cases leave that namespace empty.
8. **Bounded data.** Frame, request, output, diagnostic, active-command, process-observation, denial, and job limits are fixed. The broker drains capped output and marks it truncated. The extension retains at most 2 MiB from each background job. The macOS tracker keeps at most 4,096 process identities per command; the collector also caps raw lines, retained records, and per-command results.
9. **Explicit network rights.** Network access stays blocked unless the active project policy contains a previously approved exact hostname or IP, or `network_local` for local servers. Bash and background-job calls cannot declare rights. The host proxy enforces exact host rights. macOS grants loopback bind and connect for `network_local`; macOS loopback shares the host network. It also permits Unix socket creation and bind only at validated write roots, without Unix socket outbound access. Linux keeps local ports in a private network namespace, blocks `AF_UNIX`, keeps any proxy socket in the host launcher, and remounts its unique directory read-only after writable `/tmp`. A host grant covers all ports for that host. Trusted machine Unix socket paths stay separate from file and proxy rights and use exact Seatbelt filters.
10. **Hints do not grant.** When Seatbelt hints are available, they may explain a failed command through grouped filesystem, cache, and network diagnostics. Summaries report counts, a common/category root, and no more than three examples total; `/dev` is ignored. They never prompt or retry. Known host cache paths recommend the sanctioned `development_cache` request variant. The separate host-side `request_access` tool validates a batched durable policy change, checks giant siblings only in the net-new batch, shows one bounded exact list of net-new semantic entries, and offers only add-to-project-policy or deny. Broader approved trees retain hard-denied subtrees as carve-outs. Missing, late, unrelated, ambiguous, or `/dev` device denial data never adds access.
11. **Environment is replaced.** The child receives the filtered map in its request. It does not inherit the broker environment. The broker removes inherited proxy variables and adds its fixed proxy values only in proxy mode. On macOS, the trusted launcher also adds the fixed conceal-library path and an encoded copy of the checked read-deny rules. These values add no right.
12. **Background parity.** Each native background job uses the same policy builder and its own broker, proxy, command ID, bounded output, and stdin. It keeps the project-policy snapshot from its start; later policy updates apply only to new jobs. Session shutdown stops every job. A job has no PTY.

## Main attacks and checks

| Attack | Required control |
| --- | --- |
| Change policy, broker, or approval records | Hard read/write policy; broker path and global control roots denied |
| Consume a later policy update | Immutable rights snapshot carried on one command ID; existing jobs do not reload |
| Escape through symlink or `..` | Double normalization; nearest-existing-parent resolution; protected carve-outs |
| Forge broker output | Framed private pipe; base64 child chunks; protocol handles closed in child |
| Leave an ordinary descendant after timeout | Process-group cleanup plus start-time-checked signaling of tracker observations |
| Deliberately win the fork/reparent tracking race | Out of scope on native macOS; the survivor remains under its command's Seatbelt profile |
| Reach Docker, SSH agent, tmux, or another local service | macOS `network_local` may create but not connect to Unix sockets under write roots; existing sockets need an exact machine right, and Linux user commands cannot open host Unix sockets |
| Exfiltrate through network | Network blocked or forced through the host allowlist proxy; durable `network_local` policy requires explicit approval and can reach host loopback on macOS |
| Obtain a broad grant from an app's vague error | Denials are diagnostics only; `request_access` needs explicit typed rights and rejects large sibling-file lists in favor of one visible tree right |
| Redirect an implicit cache root with a symlink | Omit fixed cache rights reached through symlinks; broker canonicalization remains authoritative |
| Poison a shared development cache for a later build | Residual risk; use separate users or disposable homes when workspaces do not trust each other |
| Exhaust broker memory or disk | Hard frame/output/denial/job limits; no unbounded log file |
| Trigger an unsandboxed fallback | Readiness gate and no protocol field for bypass |
| Drop or bypass the macOS conceal library | Seatbelt still denies the read; the tool may see `EPERM` but cannot read the data |

## Out of scope

- A hostile host user, root process, kernel, or altered `/usr/bin/sandbox-exec`.
- Protecting the host from trusted Pi extensions, since extensions run in Pi's host process.
- Proving that macOS unified logging reports every denial.
- Interactive PTY support in the first normal-bash milestone.
- Guaranteed collection of a child that deliberately escapes its process group and the non-atomic macOS descendant tracker. Strict lifetime containment requires a stronger boundary such as a disposable VM or an entitled system service.

## Release gates

`tests/macos_release.rs` is the unsandboxed macOS broker gate. It passes with filesystem rules, blocked network and sockets, approved local test ports, environment replacement, output limits, structured denial collection for a generic application error, cancellation, timeout, shutdown, process-group cleanup, and cleanup of an observed detached child. The extension end-to-end gate also checks Bun package-manager handling for a protected optional env file, the Seatbelt backstop after dropping the conceal library, a one-run denial, active exact-file and tree project grants, local test ports, exact host and IP proxy grants over several ports, denied hosts and direct bypass, and background input, stop, and cleanup. Deliberate fast `setsid` or double-fork escape is not a macOS release assertion.

Linux has an automated ignored host release gate for read-only root mounts,
exact and fresh writable grants, hidden read denies, protected control mounts,
symlink and missing-path cases, blocked IP and host Unix sockets, user/PID
namespace behavior, the proxy bridge and direct Unix-socket denial,
`no_new_privs`, seccomp, environment replacement, malformed framing, output
bounds, cancellation, timeout, shutdown, and strict `setsid -f` and double-fork
descendant cleanup. The new proxy case has not run. The gate must pass outside
an existing sandbox on both x86_64 and aarch64 before declaring the Linux
backend production-ready.
