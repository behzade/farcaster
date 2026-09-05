# Farcaster

A native workspace for running coding-agent sessions across projects, with
support for Pi, Codex, Cursor, and OpenCode.

- Live and historical sessions, with streaming text and tool calls
- Git and Jujutsu working-copy views
- Embedded Neovim and project terminals
- Parent-child workers, a coordination notice board, and durable workgraphs

## Install

Download a build for **Linux x86_64** or **macOS arm64** (12+) from
[GitHub Releases](https://github.com/behzade/farcaster/releases).

Install and authenticate the harnesses you want to use:

| Harness | Executable | Authentication |
| --- | --- | --- |
| Pi | `pi` | `/login` in Pi |
| Codex | `codex` | Follow the harness setup |
| Cursor Agent | `agent` | `agent login` |
| OpenCode | `opencode2` | Follow the harness setup |

## Run from source

Requires Rust 1.95. Linux build dependencies are listed in the
[release workflow](.github/workflows/release.yml).

```sh
cargo run -- /path/to/project
```

Without a project argument, Farcaster opens the most recent project or the
current directory.

## Documentation

- [Usage and configuration](docs/usage.md): access modes, prompt fragments,
  built-in MCP, worker routing, and environment variables
- [Development](docs/development.md): checks, benchmarks, local crates, and packaging

Safety enforcement is delegated to each harness. Farcaster's project trust is
separate; see [access modes](docs/usage.md#access-modes).

## License

GPL-3.0-or-later. See [NOTICE.md](NOTICE.md) for source and asset attribution.
