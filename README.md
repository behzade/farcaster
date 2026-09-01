# Farcaster

Farcaster is a native GPUI desktop client for coding agents. It supports Pi,
Codex, and OpenCode through backend-specific protocol adapters.

Farcaster is GPL-3.0-or-later. See [`NOTICE.md`](NOTICE.md) for source and asset
attribution.

## Current features

- Multiple live and historical sessions across projects
- Streaming text, thinking, tool calls, retries, queues, and compaction
- Harness-native access modes
- Extension questions and permission prompts
- Git and Jujutsu working-copy views
- Embedded Neovim and project terminal surfaces
- Durable drafts, session ordering, workgraphs, and application state
- Stateless MCP access to workers and workgraphs

## Prompt fragments

Farcaster owns the manual prompt fragments in [`prompts`](prompts) and exposes
them in every harness. Type `$` to complete a fragment. Multiple fragments such
as `$simplify $commit` expand in order before Farcaster sends the prompt to Pi,
Codex, or OpenCode. Harness skills and commands remain owned by their adapters.

Farcaster does not provide a filesystem or network sandbox. The access selector
configures each harness directly:

- **Full access** disables the harness sandbox.
- **Sandboxed** uses the harness's sandbox integration.
- **Auto** routes approval requests through a model reviewer when the harness
  supports it.

Unsupported modes are omitted. For Pi, sandboxed mode leaves user-installed
sandbox extensions such as `pi-nono` active; full access sets
`PI_NONO_DISABLED=1`. Codex supports all three modes. OpenCode supports sandboxed
and full modes.

Pi settings, context files, extensions, skills, and authentication load in the
selected project directory. Farcaster does not modify Pi. The `pi` executable
must be available on `PATH` unless `FARCASTER_PI_PATH` is set.

## Provider authentication

Pi does not expose provider login through its public RPC interface, so Farcaster
cannot start `/login`. Authenticate directly in a terminal before using the
provider in Farcaster:

```sh
pi
# Then run /login in Pi.
```

Restart Farcaster after login so its Pi process reloads the credentials and
available models.

## Run

```sh
cargo run -- /path/to/project
```

The project argument is optional. Farcaster otherwise opens the most recent
project or the current directory.

## macOS bundle

```sh
make bundle-macos
open target/release/Farcaster.app
```

Signing is ad hoc by default; set `CODESIGN_IDENTITY` to use a Developer ID
identity.

Farcaster serves stateless Streamable HTTP MCP at
`http://127.0.0.1:8765/mcp`. It exposes `worker_list`, `worker_send`, and the
`workgraph_*` tools. Workers are active top-level peer agents in the same
project. `worker_send` addresses an existing peer or uses `to: "new"` to create
a fresh top-level peer with the caller's harness, model, and effort. Fresh peers
are intended only for substantial independent work; harness-native subagents
should handle delegated subtasks. Farcaster accepts MCP `2026-07-28` only and
passes the endpoint to each launched agent through its native transient
configuration; it does not create or modify a project MCP file.

Useful environment variables:

- `FARCASTER_PI_PATH`: Pi executable override
- `FARCASTER_CODEX_PATH`: Codex executable override
- `FARCASTER_OPENCODE_PATH`: OpenCode executable override
- `FARCASTER_DATA_DIR`: application database, project registry, and logs
- `FARCASTER_SHELL`: login shell override
- `FARCASTER_GIT`, `FARCASTER_JJ`, `FARCASTER_NVIM`: executable overrides

On all platforms, application data defaults to `$XDG_DATA_HOME/farcaster` or
`~/.local/share/farcaster` when `XDG_DATA_HOME` is unset.

## Check

```sh
cargo fmt --check
cargo test
cargo check
cargo clippy --all-targets -- -D warnings
```

Run the transcript benchmark with:

```sh
cargo bench --bench transcript
```
