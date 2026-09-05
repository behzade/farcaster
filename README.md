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

Farcaster saves its own project trust decisions for repository commands. These
decisions do not change a harness's trust settings. Pi project-resource trust is
checked separately when opening a Pi session; other harnesses manage their own
trust. Existing Pi trust decisions do not grant Farcaster repository access.

## Prompt fragments

Files in [`prompts`](prompts) are available in every harness. Type `$` to
complete a fragment. Fragments such as `$simplify $commit` expand in order
before submission.

## Built-in MCP

Built-in MCP is enabled by default for new sessions. It provides parent-child
workers, a project coordination notice board, and durable workgraphs. It can be
disabled under **Settings → Built-in MCP**.

When disabled, the MCP server does not bind a port. Switching it off stops the
listener and disconnects existing MCP clients; switching it on starts the server.

Up to eight child workers can be active at once. Idle children keep their sessions
for reuse without counting toward that limit. Messages to an idle child wait for
a free slot before starting another turn. Children send results explicitly with
`worker_send`; Farcaster reports child failures to the parent automatically.

### Worker task routing

**Settings → Worker tasks** lets you add, rename, or delete task definitions and
choose a harness, provider, model, and effort for each judgment level. Changes
are saved with the rest of Settings; Cancel discards edits. Model choices come
from the current project's cached harness catalogs; exact IDs can also be typed.
Changing harness clears the previous provider/model/effort to avoid mixing IDs.
An empty effort uses that backend's default.

The initial task definitions are `read`, `implement`, and `review`, each with:

| Judgment | Responsibility | Initial route (Pi / openai-codex) |
| --- | --- | --- |
| `specified` | Parent supplies the procedure or exact checks | `gpt-5.6-luna`, high |
| `guided` | Child makes local decisions within constraints | `gpt-5.6-sol`, medium |
| `independent` | Child chooses an approach or challenges assumptions | `gpt-6-astra`, high |

These are editable starter model IDs, not availability guarantees. Configure
routes for your installed harnesses and authenticated providers. Farcaster does
not inherit the parent's execution profile or silently fall back to it.
Task names classify work that is already being delegated; they are not agent
personas or permission restrictions. The schema contains no task-specific
recommendations about when to delegate.

Creating a child requires `task`; `judgment` defaults to `guided`:

```json
{"to":"check-parser","task":"review","judgment":"specified","message":"Check these three invariants…"}
```

Follow-up messages can omit both fields. A child's task, judgment, and resolved
route are bound on creation; conflicting classifications are rejected. Use a new
child name to select different routing. Deleting all definitions disables new
child creation but preserves messaging with existing children. Children cannot
select routing or spawn grandchildren. The tool schema exposes the saved task
names; clients that cache schemas may need to refresh their tools after edits.

Children may use a different harness from their parent. Farcaster persists those
family links separately from backend-native session ancestry. Harness-specific
trust, authentication, and access controls still apply; task routing does not
grant additional permissions or disable a harness's native delegation tools.

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
