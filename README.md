# Pi

This is the Pi setup I use for day-to-day coding. Nix pins the agent,
extensions, prompts, skills, and theme so I get the same setup on macOS and
Linux.

Most of the work in this repo is around shell execution. Commands run in a
native OS sandbox, extra file access goes through a user approval, and a broken
or unavailable backend fails closed.

This is a personal setup, not a turnkey Pi distribution. Machine-specific
paths, network policy, MCP servers, and notification settings live in my
separate `nix-config` repo.

## What is included

- A native sandbox broker using Seatbelt on macOS and Bubblewrap on Linux.
- Shell write approvals derived from sandbox denials; the model cannot declare
  filesystem rights.
- Async subagents with steering, timeouts, review prompts, and parent-visible
  approval requests.
- Web search, page extraction, and video tools, with OpenAI used first when the
  request is supported.
- Stateless MCP access through a pinned `mcp-cli`.
- Server-side OpenAI compaction for long sessions.
- Compact read, edit, and shell output with syntax-highlighted diffs.
- A Gruvbox dark-hard theme and small hooks for notifications, titles, user
  input, and session state.

## Repository map

- [`extensions/sandbox`](extensions/sandbox) contains the Pi adapter,
  permission UI, native broker client, Codex fallback, and background-job
  support.
- [`sandbox-broker`](sandbox-broker) contains the Rust broker and its security
  documentation.
- [`extensions/dense-tools`](extensions/dense-tools) renders compact tool output
  and side-by-side diffs.
- [`nix`](nix) contains the pinned builds for Pi and every packaged extension.
- [`patches`](patches) contains the local changes applied to third-party Pi
  extensions.
- [`APPEND_SYSTEM.md`](APPEND_SYSTEM.md) is the working contract appended to
  Pi's system prompt.
- [`skills`](skills), [`themes`](themes), and [`tests`](tests) contain the local
  skills, theme, and shared checks.

## Build and test

The flake currently targets Apple Silicon macOS and x86-64 Linux.

Build the complete agent:

```sh
nix build
```

Run the full flake checks:

```sh
nix flake check
```

The main checks can also be run directly:

```sh
npm run check --prefix extensions/sandbox
cargo test --manifest-path sandbox-broker/Cargo.toml
node --test tests/governance.test.ts tests/theme-and-rendering.test.ts
```

Individual packages are available for focused work:

```sh
nix build .#sandbox
nix build .#sandbox-broker
nix build .#dense-tools
nix build .#mcp-cli
nix build .#subagents
nix build .#openai-server-compaction
nix build .#permission-system
nix build .#web-access
```

My Home Manager configuration consumes the default flake package and deploys
it at `~/.pi/agent`.

## Sandbox

The default backend is named `native-preview` in the config. It starts one
broker per Pi session and a fresh OS sandbox for each foreground command:

- macOS uses `/usr/bin/sandbox-exec` with a generated Seatbelt profile.
- Linux uses a Nix-pinned Bubblewrap binary, a read-only host root, private
  namespaces and `/proc`, `NoNewPrivs`, and a blocked-network seccomp filter.

The default policy can read most of the host and write the workspace, temporary
directories, and a sandbox-only development cache. It keeps `.git`, project
`.pi`, Pi and Codex configuration, common credential directories, auth files,
and workspace secrets protected. The command environment is filtered, and
common package-manager caches are redirected under
`~/.cache/pi-sandbox`.

File tools ask before accessing a path outside their current rights. Shell
commands run once under the OS sandbox; when the backend reports a safe,
specific denied path, Pi can ask for that right and retry within the same tool
call. The model sees only the final attempt in its tool history. Saved rights
are scoped to the real workspace path.

Native execution is deliberately narrow:

- It supports one foreground command at a time.
- Network access is blocked.
- macOS denial hints are best effort; Linux has no structured denial source.
- Native background jobs and per-command network grants are not implemented.

The macOS release gate passes. The Linux broker is in use on x86-64, but its
ignored release test still needs to be run as a dedicated host-level gate
before treating the backend as portable beyond this setup.

To use the installed Codex CLI backend instead, set this in the global
`~/.pi/agent/extensions/sandbox.json`:

```json
{
  "backend": "codex"
}
```

The Codex backend keeps the same filesystem policy and adds sandboxed
background jobs and exact per-command network-host approvals.

Global sandbox configuration lives at
`~/.pi/agent/extensions/sandbox.json`. A project may add stricter rules in
`.pi/sandbox.json`, but project config cannot add rights, replace the broker,
or disable the sandbox.

The development cache is extensible without changing the extension:

```json
{
  "developmentCache": {
    "root": "~/.cache/pi-sandbox",
    "environment": {
      "CUSTOM_TOOL_CACHE": "custom-tool"
    }
  }
}
```

Cache paths must stay beneath the configured root. The cache is shared by
sandboxed commands across workspaces, so projects that do not trust each other
should use separate users or disposable homes.

The broker details are documented in:

- [`sandbox-broker/PROTOCOL.md`](sandbox-broker/PROTOCOL.md)
- [`sandbox-broker/THREAT_MODEL.md`](sandbox-broker/THREAT_MODEL.md)
- [`sandbox-broker/UPSTREAM.md`](sandbox-broker/UPSTREAM.md)
