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
- Trusted project-scoped Effect v4 tools loaded from `.pi/project-tools` with full host
  rights after project trust.
- A global `report_pi_feedback` tool that records concrete agent-environment
  friction in `~/.pi/agent/agent-feedback.jsonl` and notifies without blocking.
- A Gruvbox dark-hard theme and small hooks for notifications, titles, user
  input, and session state.

## Repository map

- [`extensions/sandbox`](extensions/sandbox) contains the Pi adapter,
  permission UI, native broker client, exact-host proxy, and native
  background-job support.
- [`sandbox-broker`](sandbox-broker) contains the Rust broker and its security
  documentation.
- [`extensions/dense-tools`](extensions/dense-tools) renders compact tool output
  and provides `pi-diff`, the same syntax-highlighted diff view as a terminal
  command. It keeps a bounded output cache in the system temporary directory so
  reopening the same diff at the same width skips syntax highlighting work.
- [`extensions/openai-server-compaction`](extensions/openai-server-compaction)
  owns the OpenAI compaction code. Codex compaction reuses Pi AI's cached
  WebSocket response chain when the host exposes native output items.
- [`extensions/project-tools`](extensions/project-tools) loads strict tool
  manifests and Effect v4 handlers from trusted project `.pi/project-tools`
  directories. These handlers run in Pi's host process, not the shell sandbox.
- [`extensions/agent-feedback.ts`](extensions/agent-feedback.ts) exposes the
  non-blocking feedback tool to main and packaged subagents.
- [`nix`](nix) contains the pinned builds for Pi and every packaged extension.
- [`apps/pi-terminal`](apps/pi-terminal) pins the upstream Pi 0.84.2 terminal
  client and the small Pi AI output-item hook needed for cached OpenAI
  compaction.
- [`apps/pi-gpui`](apps/pi-gpui) is a native GPUI client for Pi's public RPC
  mode. It is a distinct GPL-3.0-or-later module; the enclosing repository's
  MIT license does not replace that module's license.
- [`patches`](patches) contains the local changes applied to third-party Pi
  extensions.
- [`SYSTEM.md`](SYSTEM.md) is the base Pi system prompt. Nix fills its pinned
  Pi package path during the build.
- [`APPEND_SYSTEM.md`](APPEND_SYSTEM.md) is the terse working contract appended
  to that prompt.
- [`skills`](skills), [`themes`](themes), and [`tests`](tests) contain the local
  skills, theme, and shared checks.

`SYSTEM.md` is the active override, not a reference copy. Its single opt-in marker
asks the pinned Pi patch to append active tool snippets and guidelines after
`APPEND_SYSTEM.md`, project context, the skill catalog, and the working directory.
This tail placement keeps the static prompt prefix stable when tools change; tool
schemas remain separate. `/prompt-report` shows model-hidden size estimates and
fingerprints, while `/prompt-report full` opens the exact prompt and pre-provider
active definitions in a disposable viewer.

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

Run the native Pi client without rebuilding the Home Manager configuration:

```sh
make run
make run PROJECT=/path/to/project
```

The Rust/GPUI development shell is available with either `nix develop` or
`nix develop .#pi-gpui`; it is the default dev shell so the root `.envrc` can
use `use flake` without building the Pi agent packages.

The main checks can also be run directly:

```sh
npm run check --prefix extensions/sandbox
npm run check --prefix extensions/project-tools
cargo test --manifest-path sandbox-broker/Cargo.toml
CARGO_TARGET_DIR="$PWD/target" nix develop .#pi-gpui -c \
  cargo test --manifest-path apps/pi-gpui/Cargo.toml
CARGO_TARGET_DIR="$PWD/target" nix develop .#pi-gpui -c \
  cargo check --manifest-path apps/pi-gpui/Cargo.toml
node --test \
  tests/governance.test.ts \
  tests/output-bounds.test.ts \
  tests/prompt-contract.test.ts \
  tests/prompt-inspector.test.ts \
  tests/theme-and-rendering.test.ts \
  tests/terminal-text.test.ts
```

The full sandbox test needs a built broker and an unsandboxed host because it
must observe real OS denials and bind local network fixtures:

```sh
cargo build --manifest-path sandbox-broker/Cargo.toml
npm run check:e2e --prefix extensions/sandbox
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
nix build .#project-tools
nix build .#pi-terminal
nix build .#web-access
```

My Home Manager configuration consumes the default flake package and deploys
it at `~/.pi/agent`.

## Sandbox

The default backend is named `native-preview` in the config. It starts one
broker per Pi session and a fresh OS sandbox for each foreground command:

- macOS uses `/usr/bin/sandbox-exec` with a generated Seatbelt profile.
- Linux uses a Nix-pinned Bubblewrap binary, a read-only host root, private
  namespaces and `/proc`, `NoNewPrivs`, and a network namespace with a
  restricted loopback bridge when a host is approved.

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

- Each broker supports one command at a time. The session owns one foreground
  broker and each background job owns a separate broker.
- Network access starts blocked. A user may grant one exact hostname or IP for
  one command or save it for the workspace. A host-owned proxy enforces that
  set; the OS sandbox blocks direct bypass. A host grant applies to all ports on
  that host.
- macOS denial hints are best effort; Linux has no structured denial source.
- Background jobs support bounded output, status, input, stop, and session
  cleanup. They do not provide a PTY.

The macOS release gate and the extension's real-broker end-to-end test pass.
The Linux broker is in use on x86-64, but its ignored host release test,
including the new network bridge checks, still needs a Linux run before this
change can claim Linux network parity.

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
