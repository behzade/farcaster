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
- Stateless MCP access to workers, workgraphs, and sandbox grants

Pi settings, context files, extensions, skills, and authentication load in the
selected project directory. Farcaster owns the outer nono sandbox and runs Pi's
inner sandbox unrestricted. Farcaster does not modify Pi. The `pi` executable
must be available on `PATH`. Packaged builds use the pinned `nono` CLI sidecar
next to the Farcaster executable. Development builds otherwise resolve `nono`
from `PATH`.

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

The bundle script downloads the architecture-specific upstream `nono` v0.61.1
release when its verified archive is not already in `target/release`, verifies
its pinned SHA-256 digest, places it in `Contents/MacOS`, and signs the complete
bundle. Signing is ad hoc by default; set
`CODESIGN_IDENTITY` to use a Developer ID identity. The packaged application
does not require `PATH` or `FARCASTER_NONO_PATH` to locate `nono`.

Farcaster serves stateless Streamable HTTP MCP at
`http://127.0.0.1:8765/mcp`. It exposes `worker_backends`, `worker_start`,
`worker_send`, `worker_respond`, `worker_list`, `worker_status`, `worker_stop`,
`request_access`, and the `workgraph_*` tools. It accepts MCP `2026-07-28` only.
Approved session grants last until Farcaster exits; project grants are stored in
Farcaster application data and bound to the workspace identity. On approval,
Farcaster interrupts the pending access call, restarts the harness with the grant,
and resumes the task with an internal continuation hidden from the UI
transcript. Farcaster passes
the endpoint to each launched agent through its native transient configuration;
it does not create or modify a project MCP file.

Useful environment variables:

- `FARCASTER_PI_PATH`: Pi executable override
- `FARCASTER_NONO_PATH`: fixed nono executable override
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
