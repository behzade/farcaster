# Farcaster

Farcaster is a native GPUI desktop client for coding agents. The current
implementation starts Pi through its public JSONL RPC mode. Additional agent
backends are not implemented yet.

Farcaster is GPL-3.0-or-later. See [`NOTICE.md`](NOTICE.md) for source and asset
attribution.

## Current features

- Multiple live and historical sessions across projects
- Streaming text, thinking, tool calls, retries, queues, and compaction
- Model, thinking-level, and whole-agent sandbox controls
- Extension questions and permission prompts
- Git and Jujutsu working-copy views
- Embedded Neovim and project terminal surfaces
- Durable drafts, session ordering, workgraphs, and application state
- Stateless MCP access to workgraphs and user-approved sandbox grants

Pi settings, context files, extensions, skills, and authentication load in the
selected project directory. Farcaster owns the outer nono sandbox and runs Pi's
inner sandbox unrestricted. Farcaster does not modify Pi. The `pi` executable
must be available on `PATH`.

## Run

```sh
cargo run -- /path/to/project
```

The project argument is optional. Farcaster otherwise opens the most recent
project or the current directory.

Farcaster serves stateless Streamable HTTP MCP at
`http://127.0.0.1:8765/mcp`. It exposes `request_access`, `workgraph_search`,
`workgraph_patch`, and `workgraph_complete`, and accepts MCP `2026-07-28` only.
Approved session grants last until Farcaster exits; project grants are stored in
Farcaster application data and bound to the workspace identity. Grants activate
by restarting and resuming the agent after its current turn. Farcaster passes
the endpoint to Pi as transient launch configuration; it does not create or
modify a project MCP file.

Useful environment variables:

- `FARCASTER_PI_PATH`: Pi executable override
- `FARCASTER_NONO_PATH`: required fixed nono executable for restricted modes
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
