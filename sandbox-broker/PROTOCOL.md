# Broker Protocol v4

## Channel

Pi starts one broker as a direct child for each session. Requests use broker stdin and events use broker stdout. The sandboxed command never inherits either protocol handle. Broker stderr is for bounded host-side diagnostics only.

Each message is UTF-8 JSON with a four-byte unsigned big-endian byte count. The maximum frame size is 1 MiB. A zero, partial, oversized, malformed, unknown-field, or trailing-data frame ends the broker session. The extension then blocks shell starts. IDs, paths, arguments, and environment values may not contain NUL. Child output cannot become a broker event because the broker wraps it in a typed frame and base64 encodes its bytes.

The pipe supplies peer isolation. The broker does not open a Unix or TCP socket and does not accept an auth token from a command.

## Startup

The first server frame is `ready`:

```json
{
  "type": "ready",
  "version": 4,
  "platform": "macos",
  "backend": "seatbelt",
  "can_exec": true,
  "max_frame_bytes": 1048576
}
```

Pi checks the version, platform, backend, and `can_exec`. It blocks shell calls if startup fails, the frame times out, or `can_exec` is false. It never falls back to a plain host process.

Linux identifies itself with `platform: "linux"` and `backend: "bubblewrap"`.
The client accepts that pair only on Linux. The broker reports
`can_exec: true` only after the fixed Bubblewrap binary passes a real
user/PID/network/IPC/UTS namespace, private `/proc`, seccomp, and `NoNewPrivs`
self-test; finding a binary alone is not enough.

## Requests

### `exec`

```json
{
  "type": "exec",
  "id": "tool-call-id",
  "command": {
    "program": "/bin/bash",
    "args": ["-c", "issues search view=issue number=79"]
  },
  "cwd": "/absolute/workspace",
  "env": { "HOME": "/Users/user", "PATH": "/usr/bin:/bin" },
  "timeout_ms": 30000,
  "interactive": false,
  "policy": {
    "base_rights": [
      { "access": "read", "path": "/", "scope": "tree", "missing_path": "reject" },
      { "access": "write", "path": "/absolute/workspace", "scope": "tree", "missing_path": "reject" }
    ],
    "grants": [
      { "access": "write", "path": "/Users/user/.local/share/issues", "scope": "tree", "missing_path": "reject" }
    ],
    "denies": [
      { "access": "read_write", "pattern": "/Users/user/.ssh", "scope": "tree" },
      { "access": "read_write", "pattern": "/**/.env", "scope": "glob" }
    ],
    "network": { "mode": "blocked" },
    "unix_socket_roots": ["/nix/var/nix/daemon-socket/socket"],
    "output_limit_bytes": 10485760
  }
}
```

Rules:

- The active command ID is unique. The extension generates one fresh ID for each tool call. It never automatically retries a command.
- `program`, `cwd`, and each non-glob path are absolute. v4 permits one active command per broker; a second `exec` fails. The extension uses one broker for foreground work and a separate broker for each background job.
- `command` uses argv. The bash tool chooses `/bin/bash -c`; the broker does not parse shell text.
- `env` is the whole child environment, not a patch over the broker environment.
- `scope: file` uses an exact path. `scope: tree` uses that path and its children.
- `missing_path` is `reject`, `create_file`, or `create_tree`. It must match the scope and access. Reads always use `reject`.
- `base_rights` come from machine policy. `grants` come from the active, previously approved `.pi/extensions/sandbox/sandbox.json` project policy. Keeping project rights in `grants` permits explicit `.git` access while broker hard denies and machine denies still win.
- Denies have file, tree, or reviewed glob scope. Denies and broker hard rules win over every right.
- The broker resolves paths again, checks the nearest existing parent for a missing target, and applies its own hard denies last. Seatbelt remains the run-time control against rename and symlink races.
- An absent timeout means no deadline. Cancellation still works.
- `interactive: true` keeps stdin open for `write_stdin`. Normal foreground commands set it to false and receive EOF after the broker's start barrier opens.
- `output_limit_bytes` has a broker-set upper bound. The broker keeps draining pipes after the cap so a child cannot block on output.
- `network: {"mode":"blocked"}` denies IP networking. `{"mode":"loopback"}` permits local bind, inbound, and outbound traffic when the active project policy contains `network_local`; it does not start the host proxy. Proxy mode is `{"mode":"proxy","tcp_port":43123,"unix_socket":"/tmp/pi-native-proxy-.../proxy.sock","allow_local_binding":false}`. The extension creates both endpoints for one host-owned proxy with the active policy's exact hostname and IP set. `allow_local_binding` is true only when that policy also contains `network_local`. The broker validates the port and existing socket. On macOS, Seatbelt also permits Unix socket creation and bind only at validated writable paths; it does not grant Unix socket outbound access. On Linux, Bubblewrap keeps loopback in a private network namespace; a fixed bridge reaches only the validated proxy socket, and the user command's seccomp filter denies `AF_UNIX` sockets. The proxy supports HTTP, HTTP CONNECT, and SOCKS5. A host right has no wildcard and applies to all ports on that exact host.
- On macOS, `unix_socket_roots` contains at most 16 absolute socket paths from trusted machine config and emits exact-path Seatbelt rules. Linux rejects non-empty general socket paths. The network proxy socket is a separate field and never becomes a user-command file or socket right.

### `cancel`

```json
{ "type": "cancel", "id": "tool-call-id" }
```

The broker signals the command process group, waits for a short fixed cleanup limit, then kills what remains in that group. It also stops its best-effort macOS descendant tracker and signals observed processes whose PID and start time still match. A timeout uses the same path. Cancellation is idempotent when no command is active because an `exit` event may cross a late cancel request. A cancel for a different active ID still fails. `exit` remains the final command event. Deliberate fast `setpgid`, `setsid`, or double-fork escape from the non-atomic tracker is outside protocol v4's lifecycle guarantee.

### `write_stdin`

```json
{ "type": "write_stdin", "id": "background/server", "data_base64": "aGVsbG8K" }
```

The request writes bounded framed bytes to an active command whose `interactive`
flag is true. It fails for an unknown ID, a normal command, or a command whose
stdin has closed. Stdin bytes do not enter the command output stream.

### `shutdown`

```json
{ "type": "shutdown" }
```

The broker stops all owned children, closes its session denial collector, and exits. EOF has the same cleanup rule.

## Events and state

A command moves through `accepted -> started -> terminal`. It emits at most one `started` and exactly one terminal result. A pre-start `error` is terminal and has no `exit`. A started command ends with `exit`; broker loss is a host-side terminal error reported by the extension.

A successful start emits zero or more stream events between `started` and `exit`. The macOS backend also emits one bounded `denials` event after process and output cleanup and before `exit`; Linux emits no `denials` event. `timed_out` and `cancelled` state why the broker began termination; the exit code and signal state how the process ended:

```json
{ "type": "started", "id": "tool-call-id", "pid": 1234 }
{ "type": "stdout", "id": "tool-call-id", "sequence": 0, "data_base64": "aGVsbG8K" }
{
  "type": "denials",
  "id": "tool-call-id",
  "items": [{ "operation": "file-write-create", "path": "/state/file", "process": "issues" }],
  "complete": false
}
{
  "type": "exit",
  "id": "tool-call-id",
  "code": 1,
  "signal": null,
  "timed_out": false,
  "cancelled": false,
  "output_truncated": false
}
```

Stdout and stderr each have a zero-based sequence number. The broker preserves arbitrary bytes and caps each chunk and total output. The macOS collector keeps a session-long `/usr/bin/log stream`, waits for readiness before `ready`, and attributes records through the command's observed PIDs and sequence window. It caps raw lines, retained records, command items, and command bytes. `complete: false` states that unified logging and PID discovery can miss records, so an empty set proves nothing. Log records also lack process start times, so PID reuse can misattribute a hint. When hints are available, Pi renders filesystem, managed-cache, and network hints as grouped, bounded diagnostics with counts, a common/category root, and at most three examples total. `/dev` hints are ignored. Hints never prompt, grant access, or retry a command. Access changes happen separately through host-side `request_access`, which shows only bounded net-new semantic policy entries and conditionally updates the checked-in project policy if it has not changed during approval. Linux emits no denial hints, and the client treats their absence as expected only for the verified Linux/Bubblewrap pair.

## Grant isolation

Rights live in the immutable request for one ID; there is no shared grant queue or separate grant message. Project policy changes affect later calls only, and an existing background job keeps its start policy. A duplicate active ID fails. A command cannot cancel or change another command by writing to stdout.

## Version changes

Protocol v3 added proxy endpoints, interactive stdin, and `write_stdin`.
Protocol v4 added command-only loopback bind and connect rights.
Adding parallel commands per broker, PTY handles, or a new right form requires a
new protocol version. Strict unknown-field checks prevent silent version skew.
