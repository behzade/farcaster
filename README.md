# Pi

This is the Pi setup I use for day-to-day coding. Nix pins the agent,
extensions, prompts, skills, and theme so I get the same setup on macOS and
Linux.

Most of the work in this repo is around shell execution. Commands run in a
native OS sandbox, durable project access is approved through one host tool,
and a broken or unavailable backend fails closed.

This is a personal setup, not a turnkey Pi distribution. Machine-specific
paths, network policy, MCP servers, and notification settings live in my
separate `nix-config` repo.

## What is included

- A native sandbox broker using Seatbelt on macOS and Bubblewrap on Linux.
- Portable, checked-in project access policy with explicit host approval and no
  automatic command retries.
- Persistent child Pi sessions with forked or blank context, steering, waiting,
  model selection, and cancellation.
- Web search, page extraction, and video tools, with OpenAI used first when the
  request is supported.
- Stateless MCP access through a pinned `mcp-cli`.
- Server-side OpenAI compaction for long sessions.
- Compact read, edit, and shell output with syntax-highlighted diffs.
- First-party TypeScript host integrations use Effect v4 for async work, typed failures,
  cancellation, and resource lifetimes; pure parsers, policies, and renderers stay synchronous.
- Trusted project-scoped Effect v4 tools loaded from `.pi/project-tools` with full host
  rights after project trust.
- A global `report_pi_feedback` tool that records concrete agent-environment
  friction in `~/.pi/agent/agent-feedback.jsonl` and notifies without blocking.
- A Gruvbox dark-hard theme and small hooks for notifications, titles, user
  input, and session state.

## Repository map

- [Pi Guardian](https://github.com/behzade/pi-guardian) supplies the pinned
  sandbox, approval transport, native broker, exact-host proxy, and background
  jobs as one external extension.
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
- [`extensions/subagents`](extensions/subagents) owns persistent child Pi
  sessions. Child completion automatically reprompts the parent; three
  Effect-backed tools start, message, and control children.
- The loose entrypoints under [`extensions`](extensions) are packaged together by
  `pi-core-extensions`; notifications, title animation, user input, and feedback use the
  same pinned Effect v4 runtime while Pi callbacks remain boundary adapters. Type `$`
  at a token boundary to autocomplete prompts or skills; invocations such as
  `$simplify $commit` compose in order.
- [`extensions/agent-feedback.ts`](extensions/agent-feedback.ts) exposes the
  non-blocking feedback tool to Pi sessions.
- [`nix`](nix) contains the pinned builds for Pi and every packaged extension.
- [`apps/pi-terminal`](apps/pi-terminal) pins the upstream Pi 0.84.2 terminal
  client and the small Pi AI output-item hook needed for cached OpenAI
  compaction.
- [`apps/pi-gpui`](apps/pi-gpui) is a native GPUI client for Pi's public RPC
  mode. On macOS, the app starts the user's login shell in the project directory
  and gives the captured project environment to the RPC process. It is a
  distinct GPL-3.0-or-later module; the enclosing repository's MIT license does
  not replace that module's license.
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

## Develop and test

The project shell already provides Cargo, Node.js, and the other development
tools. Use them directly. Routine development and validation must not run Nix;
run a Nix command only when the user asks for that exact check.

Run the native Pi client without rebuilding the Home Manager configuration:

```sh
make run
make run PROJECT=/path/to/project
```

The root `.envrc` enters the default Rust/GPUI development shell and sets the
shared Cargo target directory. Do not create another development shell, Cargo
target directory, cache, or dependency folder.

Choose only the checks that cover the changed area. Start with an exact test or
package check; do not run this whole list for every change:

```sh
npm run check --prefix extensions/project-tools
npm run check --prefix extensions/subagents
make check-gpui
node --test \
  tests/governance.test.ts \
  tests/session-agents-package.test.ts \
  tests/prompt-contract.test.ts \
  tests/prompt-inspector.test.ts \
  tests/theme-and-rendering.test.ts \
  tests/terminal-text.test.ts \
  tests/user-invocations.test.ts
```

Sandbox and broker checks live in the pinned
[Pi Guardian](https://github.com/behzade/pi-guardian) repository. Its full
platform test requires an unsandboxed host because it observes real OS denials
and binds local network fixtures.

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

Shell commands and background-job starts carry no permission declarations.
Each call runs exactly once with the active policy. A denied command returns a
bounded grouped diagnostic with at most three example paths and is never
retried. Built-in file tools also deny without prompting and point the agent to
`request_access`.

`request_access` batches portable filesystem, exact-host, local-network, and
managed development-cache adapter entries. It shows one bounded exact diff of
only the net-new semantic entries, with **Add to project policy** and **Deny**
choices. Approval conditionally writes and activates that policy for later
commands; a concurrent/manual edit aborts rather than being overwritten, and
the agent must explicitly rerun a command. Existing background jobs
keep the immutable policy captured at start. Trusted project `.pi/project-tools`
remain host tools and do not go through the command broker.

Native execution is deliberately narrow:

- Each broker supports one command at a time. The session owns one foreground
  broker and each background job owns a separate broker.
- Network access starts blocked. Project policy may grant one exact hostname or
  IP, enforced by a host-owned proxy, or `network_local` for local servers. A
  host grant applies to all ports on that exact host. Linux keeps local ports in
  the command's private network namespace. macOS uses the host loopback
  interface and permits Unix socket bind only at paths the command may write.
- When macOS denial hints are available, summaries are grouped and best effort;
  Linux has no structured denial source. Hints are diagnostics only and never
  prompt or grant access.
- Background jobs support bounded output, status, input, stop, and session
  cleanup. They do not provide a PTY.

The macOS release gate and the extension's real-broker end-to-end test pass.
The Linux broker is in use on x86-64, but its ignored host release test,
including the new network bridge checks, still needs a Linux run before this
change can claim Linux network parity.

Global machine hard policy lives at
`~/.pi/agent/extensions/sandbox.json`. A trusted project's portable access
policy is checked in at `.pi/extensions/sandbox/sandbox.json`. Keeping it under
Pi's project-extension root makes stock Pi require project trust before loading it:

```json
{
  "version": 1,
  "rights": [
    { "kind": "filesystem", "access": "write", "path": ".git", "scope": "tree" },
    { "kind": "network_host", "host": "registry.npmjs.org" },
    { "kind": "network_local" }
  ],
  "developmentCache": {
    "environment": { "CUSTOM_TOOL_CACHE": "custom-tool" }
  }
}
```

Relative filesystem paths resolve from the project root and `~/` paths resolve
from the current user's home. Checked-in absolute paths are rejected;
`request_access` converts absolute denial paths beneath the workspace or home to
portable form and rejects all others. Machine denies, broker hard rules,
secrets, project `.pi` writes, and any filesystem right crossing an existing
symlink always lose. Grant paths are revalidated immediately before every
broker request, so a missing approved path cannot later retarget through a
symlink. Direct policy-file edits are loaded only on a later session; explicitly
approved `request_access` updates activate immediately.

Development caches share one managed namespace under the machine-configured
root (by default `~/.cache/pi-sandbox`). Projects may add safe environment
mappings with relative targets beneath that root, but cannot relocate it. The
cache is shared across workspaces, so projects that do not trust each other
should use separate users or disposable homes.

The broker protocol, threat model, and upstream notes are maintained in
[Pi Guardian](https://github.com/behzade/pi-guardian/tree/main/sandbox-broker).
