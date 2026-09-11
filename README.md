<img src="assets/brand/farcaster.svg" alt="Farcaster app icon" width="128" height="128">

# Farcaster

Farcaster is a native, keyboard-first workspace for running and coordinating coding agents across harnesses, with an embedded terminal and Neovim.

![Farcaster showing concurrent agent sessions, a conversation, and changed files](https://github.com/user-attachments/assets/87e034bd-d091-4820-8dc3-f81c76fcabca)

## Why

The agent space is changing fast, but switching harnesses shouldn't mean relearning the interface or leaving your editor behind. I wanted to use different agents and subscriptions with the same controls, and keep Neovim at the center of my workflow.

## Is this another agent harness?

No. Farcaster brings your existing harnesses and subscriptions into one UI. It builds around them rather than competing with them.

## What does this app give you?

- A native, keyboard-first desktop app.
- An agent, editor, and terminal for each session.
- Integrated [Neovim](#neovim) for reviewing code, editing, and sending context to agents.
- [Shared tools](#tools) for coordinating workers and tracking tasks across harnesses.

## Neovim

![Reviewing Rust code in Farcaster's embedded Neovim editor](https://github.com/user-attachments/assets/37192030-73b5-4b31-a8f8-72e3c1b8b4e9)

The app embeds a full terminal emulator using libghostty. Each chat session has its own Neovim and terminal tab. I find this a pretty nice way to work with AI as I can think of each session as a separate row of `agent | editor | terminal`.

The following integrations with Neovim are built into the app:

- Clicking on any changed file opens it in Neovim at the changed line.
- You can open the transcript itself in a temp Markdown file in Neovim to copy/select things.
- You can send messages with context from Neovim (normal mode -> line context, select -> selected context) to the current session, another existing session or a new session to fire off a new task.
- Agents can present their work to be reviewed in Neovim with filenames/lines and context using a built-in tool. ([`submit_review`](#review))

## Tools

Farcaster comes with an optional MCP server (can be turned off in settings) that provides the following capabilities:

- **Workgraph:** A set of tools (`workgraph_*`) for creating tasks and dependencies that persist beyond any one agent session.
- **Worker Tools:** Tools for creating and communicating with agent sessions across harnesses, providers, and models.

  ![A main agent coordinating workers across harnesses and models in Farcaster](https://github.com/user-attachments/assets/5d216a2b-2fef-459c-911d-20450e87d749)

- <a name="review"></a>**Review:** A tool (`submit_review`) that lets agents present files, line ranges, and notes for you to navigate and review using Neovim's quickfix list.

## Current status

I have been using Farcaster daily for a while. It's still early, and bugs/missing features are expected. The current list of supported harnesses is as follows:

- [Codex](https://github.com/openai/codex) (through codex-cli app server)
- [Pi](https://github.com/badlogic/pi-mono) (through Pi RPC)
- [OpenCode](https://opencode.ai/) (through the `opencode2` server executable)
- [Cursor](https://cursor.com/cli) (through Cursor CLI ACP adapter)
- [Claude Code](https://github.com/anthropics/claude-code) (through `claude -p`)
- Antigravity (through its ACP server)

See the [harness feature table](docs/harnesses.md) for capabilities and usage notes.

In theory, any harness supporting ACP could be integrated through the app's ACP adapter, but most harnesses support a wider range of features than ACP.

## Getting Started

Download the latest build for your platform from the [releases page](https://github.com/behzade/farcaster/releases). Builds for macOS ARM64 and x86_64 Linux are currently available.

The macOS app uses ad hoc signing; it is not notarized.
If macOS blocks the app because it is not notarized and you trust the download, follow [Apple's instructions](https://support.apple.com/102445) to allow it through System Settings → Privacy & Security → Open Anyway.

You'll also need the agent harness you want to use installed and signed in. To use Neovim in the embedded terminal, it must be available in your environment.

## Development

The repository includes a Nix development shell. To build and run from a local checkout:

```sh
nix develop
cargo run --locked --bin farcaster
```

## Why build around harnesses?

After a few attempts (tried to fork and extend codex-cli first, then tried building on Pi) I realized that the harness space is very fast moving, with a lot of work and innovation happening in the space. So it would be better to position the app in a way that benefits from said improvements instead of competing with them.

## Documentation

- [Keyboard shortcuts](docs/shortcuts.md)
- [Harness feature table](docs/harnesses.md)

## Thanks

Thanks to [Zed](https://zed.dev/) for [GPUI](https://gpui.rs/), [GPUI Component](https://github.com/longbridge/gpui-component) for the UI components, and [T3 Code](https://github.com/pingdotgg/t3code) for the UI inspiration.
