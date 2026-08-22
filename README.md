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

- Pi Guardian's native nono sandbox with explicit project/session approval and
  no automatic command retries.
- Persistent child Pi sessions with forked or blank context, steering, waiting,
  model selection, and cancellation.
- A first-party Codex Web Search tool using the active Codex login and Codex's
  direct search endpoint, with bounded cited output and no provider router.
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

- [Pi Guardian](https://github.com/behzade/pi-guardian) supplies the complete
  pinned sandbox, approval policy, and background-job security boundary. This
  repository only installs and configures its packaged extension.
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
- [`extensions/codex-web-search.ts`](extensions/codex-web-search.ts) exposes
  one compact Codex Web Search tool; direct request shaping and response
  validation live in `codex-web-search-core.ts`.
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
  mode. On Linux and macOS, the packaged desktop app starts the user's login
  shell in the project directory and gives the captured project environment to
  the RPC process. It is a distinct GPL-3.0-or-later module; the enclosing
  repository's MIT license does not replace that module's license.
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
package check; do not run this whole list for every change. Nix packaging and
lock-input changes are the exception: use `make check-flake` to build every
flake check before publishing them. Update Guardian with `make update-guardian`;
it updates the input and immediately runs that gate.

```sh
npm run check --prefix extensions/project-tools
npm run check --prefix extensions/subagents
make check-gpui
node --test \
  tests/governance.test.ts \
  tests/codex-web-search.test.ts \
  tests/session-agents-package.test.ts \
  tests/prompt-contract.test.ts \
  tests/prompt-inspector.test.ts \
  tests/theme-and-rendering.test.ts \
  tests/terminal-text.test.ts \
  tests/user-invocations.test.ts
```

Sandbox implementation checks and security documentation live in the pinned
[Pi Guardian](https://github.com/behzade/pi-guardian) repository. This repository
keeps only package-layout and Pi compatibility checks.

My Home Manager configuration consumes the default flake package and deploys
it at `~/.pi/agent`.

## Sandbox

[Pi Guardian](https://github.com/behzade/pi-guardian) owns sandbox enforcement,
policy, approvals, fixed native executables, and background jobs. This repository
pins and installs the finished Guardian package without rebuilding its internals.

Machine-specific Guardian policy remains in the separate `nix-config` repository.
See Guardian's README for configuration, supported rights, threat model, and
platform status.
