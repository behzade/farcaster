# Broker Protocol v1

## Channel

Pi starts one broker as a direct child for each session. Requests use broker stdin and events use broker stdout. The sandboxed command never inherits either protocol handle. Broker stderr is for bounded host-side diagnostics only.

Each message is UTF-8 JSON with a four-byte unsigned big-endian byte count. The maximum frame size is 1 MiB. A zero, partial, oversized, malformed, unknown-field, or trailing-data frame ends the broker session. The extension then blocks shell starts. IDs, paths, arguments, and environment values may not contain NUL. Child output cannot become a broker event because the broker wraps it in a typed frame and base64 encodes its bytes.

The pipe supplies peer isolation. The broker does not open a Unix or TCP socket and does not accept an auth token from a command.

## Startup

The first server frame is `ready`:

```json
{
  "type": "ready",
  "version": 1,
  "platform": "macos",
  "backend": "seatbelt",
  "can_exec": true,
  "max_frame_bytes": 1048576
}
```

Pi checks the version, platform, backend, and `can_exec`. It blocks shell calls if startup fails, the frame times out, or `can_exec` is false. It never falls back to a plain host process.

A future Linux implementation of this same protocol version must identify itself with `platform: "linux"` and `backend: "bubblewrap"`. The current broker reports `can_exec: false` on Linux, and the current client does not accept that pair yet. Linux support must add a real bubblewrap namespace self-test before readiness; the presence of a binary alone is not enough. See [LINUX_BACKEND.md](LINUX_BACKEND.md).

## Requests

### `exec`

```json
{
  "type": "exec",
  "id": "tool-call-id/attempt-0",
  "command": {
    "program": "/bin/bash",
    "args": ["-c", "issues search view=issue number=79"]
  },
  "cwd": "/absolute/workspace",
  "env": { "HOME": "/Users/user", "PATH": "/usr/bin:/bin" },
  "timeout_ms": 30000,
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
    "output_limit_bytes": 10485760
  }
}
```

Rules:

- The active command ID is unique. The extension generates a fresh ID for each call and retry.
- `program`, `cwd`, and each non-glob path are absolute. v1 permits one active command; a second `exec` fails.
- `command` uses argv. The bash tool chooses `/bin/bash -c`; the broker does not parse shell text.
- `env` is the whole child environment, not a patch over the broker environment.
- `scope: file` uses an exact path. `scope: tree` uses that path and its children.
- `missing_path` is `reject`, `create_file`, or `create_tree`. It must match the scope and access. Reads always use `reject`.
- `base_rights` come from host policy. `grants` are rights Pi already showed and the user approved for this command ID.
- Denies have file, tree, or reviewed glob scope. Denies and broker hard rules win over every right.
- The broker resolves paths again, checks the nearest existing parent for a missing target, and applies its own hard denies last. Seatbelt remains the run-time control against rename and symlink races.
- An absent timeout means no deadline. Cancellation still works.
- `output_limit_bytes` has a broker-set upper bound. The broker keeps draining pipes after the cap so a child cannot block on output.
- Protocol v1 accepts only `network: {"mode":"blocked"}` and no Unix socket rights. Exact hosts need a later host-owned allowlist proxy and protocol version.

### `cancel`

```json
{ "type": "cancel", "id": "tool-call-id/attempt-0" }
```

The broker signals the command process group, waits for a short fixed cleanup limit, then kills what remains in that group. It also stops its best-effort macOS descendant tracker and signals observed processes whose PID and start time still match. A timeout uses the same path. Cancellation is idempotent when no command is active because an `exit` event may cross a late cancel request. A cancel for a different active ID still fails. `exit` remains the final command event. Deliberate fast `setpgid`, `setsid`, or double-fork escape from the non-atomic tracker is outside protocol v1's lifecycle guarantee.

### `shutdown`

```json
{ "type": "shutdown" }
```

The broker stops all owned children and exits. A later collector stage must close its collector here too. EOF has the same cleanup rule.

## Events and state

A command moves through `accepted -> started -> terminal`. It emits at most one `started` and exactly one terminal result. A pre-start `error` is terminal and has no `exit`. A started command ends with `exit`; broker loss is a host-side terminal error reported by the extension.

A successful start emits zero or more stream events between `started` and `exit`. `timed_out` and `cancelled` state why the broker began termination; the exit code and signal state how the process ended. The `denials` shape below is reserved for the later collector stage and v1 brokers do not emit it yet:

```json
{ "type": "started", "id": "tool-call-id/attempt-0", "pid": 1234 }
{ "type": "stdout", "id": "tool-call-id/attempt-0", "sequence": 0, "data_base64": "aGVsbG8K" }
{
  "type": "denials",
  "id": "tool-call-id/attempt-0",
  "items": [{ "operation": "file-write-create", "path": "/state/file", "process": "issues" }],
  "complete": false
}
{
  "type": "exit",
  "id": "tool-call-id/attempt-0",
  "code": 1,
  "signal": null,
  "timed_out": false,
  "cancelled": false,
  "output_truncated": false
}
```

Stdout and stderr each have a zero-based sequence number. The broker preserves arbitrary bytes and caps each chunk and total output. `complete: false` states that an empty macOS denial set proves nothing. Pi may use an exact safe path as an approval hint. It may not infer a broad root or retry with more rights without user approval.

## Grant isolation

Rights live in the immutable request for one ID; there is no shared one-time-right queue or separate grant message. A duplicate active ID fails. A command cannot cancel or change another command by writing to stdout.

## Version changes

Adding proxy ports, Unix sockets, parallel execution, PTY handles, or a new right form requires a new protocol version. Strict unknown-field checks prevent silent version skew.
