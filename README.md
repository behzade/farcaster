# Farcaster

Farcaster is a native workspace for running multiple coding-agent sessions across
projects. It supports Pi, Codex, Cursor, and OpenCode.

Farcaster is GPL-3.0-or-later. See [`NOTICE.md`](NOTICE.md) for source and asset
attribution.

## Features

- Live and historical sessions across projects
- Streaming text, reasoning, tool calls, retries, queues, and compaction
- Git and Jujutsu working-copy views
- Embedded Neovim and project terminals
- Durable drafts, session ordering, workgraphs, and application state
- Harness permission prompts, questions, models, and access modes
- Built-in tools for parent-child workers and workgraphs

## Install

Release builds are available for Linux x86_64 and macOS arm64 from
[GitHub Releases](https://github.com/behzade/farcaster/releases). macOS requires
version 12 or later.

Install and authenticate any agent harnesses you intend to use:

- Pi: `pi` (run `/login` before launching Farcaster)
- Codex: `codex`
- Cursor Agent: `agent` (run `agent login`)
- OpenCode: `opencode2`

Executable paths can be overridden with the environment variables listed under
[Configuration](#configuration).

## Run from source

Farcaster requires Rust 1.95.

```sh
cargo run -- /path/to/project
```

The project argument is optional. Farcaster otherwise opens the most recent
project or the current directory. Linux build dependencies are listed in the
[release workflow](.github/workflows/release.yml).

## Access modes

Safety enforcement is delegated to the selected harness:

- Pi: Sandboxed or Full. Sandboxed preserves installed sandbox extensions such
  as `pi-nono`; Full sets `PI_NONO_DISABLED=1`.
- Codex: Sandboxed, Auto, or Full. Auto uses model-reviewed approvals.
- Cursor and OpenCode: Sandboxed or Full.

Unsupported modes are omitted from the selector.

## Prompt fragments

Files in [`prompts`](prompts) are available in every harness. Type `$` to
complete a fragment. Fragments such as `$simplify $commit` expand in order
before submission.

## Built-in MCP

Built-in MCP is enabled by default for new sessions. It provides parent-child
workers, a project coordination notice board, and durable workgraphs. It can be
disabled under **Settings → Built-in MCP**.

## Configuration

- `FARCASTER_PI_PATH`: Pi executable
- `FARCASTER_CODEX_PATH`: Codex executable
- `FARCASTER_CURSOR_PATH`: Cursor Agent executable
- `FARCASTER_OPENCODE_PATH`: OpenCode executable
- `FARCASTER_PI_TITLE_MODEL`: Pi model for automatic session titles
- `FARCASTER_CODEX_TITLE_MODEL`: Codex model for automatic session titles
- `FARCASTER_DATA_DIR`: application database, project registry, and logs
- `FARCASTER_SHELL`: login shell
- `FARCASTER_GIT`, `FARCASTER_JJ`, `FARCASTER_NVIM`: tool executables

Application data defaults to `$XDG_DATA_HOME/farcaster`, or
`~/.local/share/farcaster` when `XDG_DATA_HOME` is unset. Run `make logs` to read
the application log.

## Development

```sh
make check
cargo bench --bench transcript
```

Create a native bundle with Cargo Packager 0.11.8:

```sh
cargo install cargo-packager --version 0.11.8 --locked
make bundle
```
