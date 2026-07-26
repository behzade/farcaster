# Pi

This repo owns Behzad's reviewed Pi coding-agent setup. It contains local
extensions, a Codex sandbox adapter, background job support, tests, and pinned
Nix builds for third-party extensions.

Machine policy stays in `nix-config`. That repo sets sandbox paths, network
hosts, and notification settings. This repo owns code and package pins.

## Layout

- `extensions/sandbox`: fail-closed `codex sandbox` adapter and IO permission
  prompt.
- `extensions/*.ts`: local notification, input, session, and title hooks.
- `skills/background-jobs`: non-blocking shell job control.
- `nix`: pinned package builds for the sandbox, subagents, and server
  compaction.
- `tests`: shared policy and notification checks.

## Checks

```sh
npm run check --prefix extensions/sandbox
node --test tests/governance.test.ts
nix flake check
```

Build one extension with:

```sh
nix build .#sandbox
nix build .#subagents
nix build .#openai-server-compaction
```

`nix-config` consumes this repo as a flake input and deploys the source and
built packages through Home Manager.

## Sandbox backend

The sandbox extension requires an installed Codex CLI with the `codex sandbox`
command. It builds a fresh Codex permission profile for each shell call from
`sandbox.json`. Every interpreter and child process keeps the same profile. A
failed Codex check blocks shell commands.

The default rights are:

- read most of the system;
- deny reads and writes for `.env` and `.key` files and for `.ssh`, `.aws`,
  and `.gnupg`; keep PEM certificate bundles readable but read-only;
- keep `~/.pi/agent` and `~/.codex` read-only so a grant cannot change its own
  policy;
- write the workspace, `/tmp`, and the system temp folder;
- reach a small set of public package and source hosts;
- block local and private network targets.

The model can call `request_io_permission` for an exact file or folder read,
an exact file or folder write, public web access, or local network access. Pi
asks the user to allow it once, always for this workspace, or not at all. A
denial can include a comment. Codex cannot limit localhost access to one
destination port, so a request for one port says that approval opens every
localhost port and may reach private-network or link-local targets. Saved rights live in
`~/.pi/agent/io-permissions.json`, keyed by the real workspace path. A one-time
right lasts for that shell call and any child process which remains alive from
it.

Workspace write access includes creating, changing, and deleting workspace
files. The sandbox does not inspect command text, so it does not add a second
prompt for a delete command. Codex keeps repository control data such as
`.git` read-only, but use version control or backups for other workspace files.

Enabling an MCP service also needs user approval because an MCP server can read
private data or act outside the shell sandbox. That approval is scoped to the
named service and can be saved for one workspace.

Unix socket access grants a right to another service. That service may do work
outside the caller's file or web limits. The machine config allows only the Nix
daemon and Pi's own tmux socket. Keep other service sockets blocked unless they
form part of the intended trust boundary.

Pi's built-in recursive `grep` and `find` tools do not run as child processes
of the sandbox. The extension blocks them and tells the agent to use `rg` or
`fd` through bash, where Codex enforces the same file policy.

Global sandbox config lives at `~/.pi/agent/extensions/sandbox.json`. A trusted
project can add tighter rules at `.pi/sandbox.json`; project config cannot add
rights or turn the sandbox off. Set `network.allowLocalNetwork` in the global
file only if every Pi session should reach localhost and private-network targets
or link-local targets without a prompt.
