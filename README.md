# Farcaster

Farcaster is a native GPUI desktop client for coding agents. The current
implementation starts Pi through its public JSONL RPC mode. Additional agent
backends are not implemented yet.

Farcaster is GPL-3.0-or-later. See [`NOTICE.md`](NOTICE.md) for source and asset
attribution.

## Current features

- Multiple live and historical sessions across projects
- Streaming text, thinking, tool calls, retries, queues, and compaction
- Model, thinking-level, and sandbox controls
- Extension questions and permission prompts
- Git and Jujutsu working-copy views
- Embedded Neovim and project terminal surfaces
- Durable drafts, session ordering, workgraphs, and application state
- Stateless MCP access to workgraph search, patch, and completion

Pi settings, context files, extensions, skills, authentication, and sandbox
behavior load in the selected project directory. Farcaster does not modify Pi.
The `pi` executable must be available on `PATH`.

## Run

```sh
cargo run -- /path/to/project
```

The project argument is optional. Farcaster otherwise opens the most recent
project or the current directory.

Farcaster serves stateless Streamable HTTP MCP at
`http://127.0.0.1:8765/mcp`. It exposes `workgraph_search`, `workgraph_patch`,
and `workgraph_complete` and accepts MCP `2026-07-28` only.

Useful environment variables:

- `FARCASTER_PI_PATH`: Pi executable override
- `FARCASTER_DATA_DIR`: application database, project registry, and logs
- `FARCASTER_SHELL`: login shell override
- `FARCASTER_GIT`, `FARCASTER_JJ`, `FARCASTER_NVIM`: executable overrides

On macOS, application data defaults to
`~/Library/Application Support/Farcaster`. Other platforms use
`$XDG_DATA_HOME/farcaster` or `~/.local/share/farcaster`.

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
