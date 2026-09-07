# Farcaster

Farcaster is a native desktop app for controlling different AI agent harnesses through one UI, with keyboard controls, an embedded terminal and Neovim.

I wanted to utilize all my agent subscriptions in the same app, with the same keybindings, and review them in Neovim.

## What you can do

### Use one UI across harnesses

Work with different agents without learning a new interface for each one. As a neovim user, I want my workspace to be routine and predictable.

### Work from the keyboard

This app is designed to make most actions comfortable without reaching for the mouse.

### Review and edit in Neovim

The app embeds a full terminal emulator using libghostty. Neovim is the default configured editor in app using the same terminal. Clicking on changed files in transcript or the git change list opens them in neovim instead of a limited diff viewer.

### Coordinate agents across harnesses

An optional MCP server gives supported harnesses access to shared tools:

- **Workgraph:** A persistent plan and task queue that lasts beyond a single session.
- **Worker tools:** Tools for creating and communicating with agent sessions across harnesses, providers, and models.

This allows harnesses like pi that don't have builtin subagents to offload implementation or review to separate workers.

### Better visibility to agent actions

Since the tasks are managed outside any one harness, they are easy to audit and change. Jumping to any subagent thread is also trivial because of the same reason.

## Current status

I have been using farcaster daily for a while. It's still early, and bugs/missing features are expected. The current list of supported harnesses is as follows:

- [Codex](https://github.com/openai/codex) (through codex-cli app server)
- [Pi](https://github.com/badlogic/pi-mono) (through pi rpc)
- [Opencode](https://opencode.ai/) (through opencode2 server)
- [Cursor](https://cursor.com/cli) (through cursor cli acp adapter)

In theory, any harness supporting acp is already supported by the app, but most harnesses support a wider range of features than the acp.

## Getting Started

Download the latest build for your platform from the [releases page](https://github.com/behzade/farcaster/releases). Builds for macOS ARM and x86 Linux are currently available.

You'll also need the agent harness you want to use installed and signed in. To use Neovim in the embedded terminal, it must be available in your environment.

## Development

The repository includes a Nix development shell. To build and run from a local checkout:

```sh
nix develop
cargo run --locked --bin farcaster
```

## How I use it

I spawn many concurrent sessions, mostly as I notice/remember issues or tasks; because of the concurrency, I don't care too much about model speed, and I make use of cheaper models as workers for more intelligent top-level agents to save costs. I try to stay in app as agents are working, notice issues in their implementation by the files they are touching, occasionally dropping into neovim to audit/change things.

## Why build around harnesses?

After a few attempts (tried to fork and extend codex-cli first, then tried building on pi) I realized that the harness space is very fast moving, with a lot of work and innovation happening in the space. So it would be better to position the app in a way that benefits from said improvements instead of competing with them.

## Documentation

- [Keyboard shortcuts](docs/shortcuts.md)
- [Harness feature table](docs/harnesses.md)

## Thanks

Thanks to [Zed](https://zed.dev/) for [GPUI](https://gpui.rs/), [GPUI Component](https://github.com/longbridge/gpui-component) for the UI components, and [T3 Code](https://github.com/pingdotgg/t3code) for the UI inspiration.
