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

The extension blocks native activation. Protocol v1 has no network, Unix socket, background-job, or denial-collector support. Process-group cleanup and bounded pipe draining have landed. A hostile child can still leave that group with `setpgid`, `setsid`, or a double fork. Codex's `kqueue` plus `proc_listchildpids` tracker is best effort and cannot close that race. Public unprivileged macOS APIs do not provide a kill-and-reap container for such children; creating a new kernel coalition fails with `EPERM` for a normal user process. Native release therefore needs a different ownership boundary, such as a disposable VM or an approved privileged owner, or an explicit reduction of the threat model. The rules below remain release requirements.

## Security rules

1. **Fail closed.** Broker startup, protocol, policy, proxy, Seatbelt, bubblewrap, or child-start failure blocks the command. No request can select an unsandboxed mode.
2. **Fresh rights.** Each command gets a new OS sandbox. One-time rights occur only in that command's immutable request and command ID.
3. **Two checks.** TypeScript resolves paths for UI. Rust resolves them again against the request cwd, canonicalizes existing ancestors, rejects relative paths, and applies hard denies last.
4. **Protected control state.** Commands cannot write the broker binary, broker policy, global `~/.pi`, global `~/.codex`, or auth and secret roots. Base workspace writes exclude `.git` and project `.pi`; only an exact approved command grant may add those project roots.
5. **No path alias escape.** Existing symlinks resolve before policy build. For a missing leaf, the broker resolves the nearest existing ancestor and appends checked normal components. Tests cover symlinks, `..`, missing paths, and protected children under broad roots.
6. **Private control channel.** Commands inherit only their stdin/stdout/stderr and needed job handles. They do not inherit broker protocol handles or a public control socket.
7. **Whole-tree control.** Before release, the backend must register a kernel-owned command boundary before user code crosses its launch barrier. Cancel, timeout, shutdown, and broker failure must stop that whole boundary and prove it empty before a terminal event. The current process group is only a fast cleanup layer and does not meet this rule by itself.
8. **Bounded data.** Frame, request, output, diagnostic, active-command, and later denial/job limits are fixed. The broker drains capped output and marks it truncated.
9. **Explicit local service rights.** Network starts blocked. A later approved-host stage must use a host-owned allowlisting proxy. A later Unix socket stage must keep roots separate and must never give normal bash the background-job control socket.
10. **Hints do not grant.** A later Seatbelt denial collector may explain one exact right. Missing, late, unrelated, or ambiguous denial data never adds access.
11. **Environment is replaced.** The child receives the filtered map in its request. It does not inherit the broker environment. The broker adds only fixed status markers and later required proxy values.
12. **Background parity.** Before native background jobs ship, they must use the same policy builder. Their one-time rights must last for that job's sandbox only and never enter another job.

## Main attacks and checks

| Attack | Required control |
| --- | --- |
| Change policy, broker, or approval records | Hard read/write policy; broker path and global control roots denied |
| Consume a sibling's one-time right | No shared grant queue; rights carried on one command ID |
| Escape through symlink or `..` | Double normalization; nearest-existing-parent resolution; protected carve-outs |
| Forge broker output | Framed private pipe; base64 child chunks; protocol handles closed in child |
| Leave a daemon after timeout | Release-blocking kernel owner plus `setpgid`, `setsid`, and double-fork tests; the current process group alone is insufficient |
| Reach Docker, SSH agent, tmux, or another local service | Unix sockets denied unless listed; reserved job socket always denied to normal bash |
| Exfiltrate through network | Network blocked or forced through host allowlist proxy; no broad local targets |
| Obtain a broad grant from an app's vague error | Explicit preflight right or one exact safe denial hint; no prose guessing |
| Exhaust broker memory or disk | Hard frame/output/denial/job limits; no unbounded log file |
| Trigger an unsandboxed fallback | Readiness gate and no protocol field for bypass |

## Out of scope

- A hostile host user, root process, kernel, or altered `/usr/bin/sandbox-exec`.
- Protecting the host from trusted Pi extensions, since extensions run in Pi's host process.
- Proving that macOS unified logging reports every denial.
- Interactive PTY support in the first normal-bash milestone.

## Release gates

The extension must not switch a platform to this broker until integration tests show filesystem rules, network mode, sockets, environment replacement, output limits, cancellation, timeout, shutdown, and child cleanup on that platform. `tests/macos_release.rs` is the unsandboxed macOS gate and includes `setpgid` and `setsid`/double-fork fixtures. It must pass without its emergency fixture cleanup before activation. macOS may ship first. Linux stays on the old fail-closed backend until the bubblewrap gate passes.
