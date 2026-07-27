# Pi

This repo owns Behzad's reviewed Pi coding-agent setup. It contains local
extensions, Codex and native sandbox code, background job support, tests, and
pinned Nix builds.

Machine policy stays in `nix-config`. That repo sets sandbox paths, network
hosts, and notification settings. This repo owns code and package pins.

## Layout

- `extensions/sandbox`: fail-closed sandbox adapter, IO permission prompt,
  native broker client, and background-job tool and helper.
- `sandbox-broker`: protocol, threat model, provenance, and the first native
  macOS Seatbelt backend.
- `extensions/*.ts`: local notification, input, session, title, and dense tool
  rendering hooks.
- `themes/gruvbox-dark-hard.json`: Gruvbox's canonical dark-hard palette for
  Pi. The palette values come from
  [morhetz/gruvbox](https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim).
  Its token roles follow Zenbones' contrast-first hierarchy: neutral lightness
  carries most structure, while hue marks links, active UI, state, and diffs.
- `nix`: pinned package builds for the sandbox, subagents, and server
  compaction.
- `tests`: shared policy and notification checks.

## Checks

```sh
npm run check --prefix extensions/sandbox
cargo test --manifest-path sandbox-broker/Cargo.toml
node --test tests/governance.test.ts
nix flake check
```

Build one extension with:

```sh
nix build .#sandbox
nix build .#sandbox-broker
nix build .#dense-tools
nix build .#subagents
nix build .#openai-server-compaction
```

`nix-config` consumes this repo as a flake input and deploys the source and
built packages through Home Manager. It must deploy the `dense-tools` flake
package to Pi's extension directory, link `themes/gruvbox-dark-hard.json` to
Pi's theme directory, and set Pi's theme to `gruvbox-dark-hard`.

## Dense tool display

`extensions/dense-tools` removes the padded shell from the built-in file tools.
It combines each run of adjacent `read` calls into one display block; Ctrl+O
still expands the grouped file contents. Its edit renderer uses a pinned
`@pierre/diffs` bundle for Shiki syntax highlighting, inline change marks, and
a responsive before/after view at 120 columns or wider. Added and removed lines
use low-intensity backgrounds blended from Gruvbox's canonical hard background
and bright green/red; changed words use stronger blends of the same colors.
The sandbox remains the only owner of `bash` and applies the same dense shell to
its bash tool, so the two extensions do not conflict.

## Sandbox backend

The sandbox extension defaults to the installed Codex CLI and its `codex
sandbox` command. It builds a fresh Codex permission profile for each shell
call from `sandbox.json`. Every interpreter and child process keeps the same
profile. A failed backend check blocks shell commands.

The tree also contains a macOS native preview, but the extension blocks
activation until the full unsandboxed macOS integration gate passes and a
kernel-owned boundary can clean hostile `setpgid`, `setsid`, and double-fork
children. A process group and Codex's best-effort PID tracker cannot provide
that guarantee. The broker has a separate pinned Nix package. On macOS, the extension
package pins that exact broker store path, but the release gate prevents it
from starting. Linux keeps an unavailable placeholder. A custom `brokerPath`
must be absolute and can come only
from global config; project config cannot switch backends or replace the broker.
Protocol v1 keeps network and Unix sockets blocked and has no native background
jobs or denial collector. `backend: "codex"` remains the only released backend.

The default rights are:

- read most of the system;
- deny reads and writes for `.env` and `.key` files and for `.ssh`, `.aws`,
  and `.gnupg`; deny reads of Pi and Codex `auth.json`; keep PEM certificate
  bundles readable but read-only;
- keep `~/.pi` and `~/.codex` read-only so a grant cannot change its own
  policy;
- write the workspace, `/tmp`, and the system temp folder;
- pass only core shell variables such as `PATH`, `HOME`, locale, and temp paths;
- remove variables whose names contain `KEY`, `SECRET`, or `TOKEN`;
- block every network host until the user grants that exact hostname or IP;
- block local and private network targets.

When `read`, `write`, `edit`, or `ls` targets a path outside the current IO
rights, the sandbox pauses that tool call and asks the user to allow it once,
always for this workspace, or not at all. A denial can include a comment. The
model does not receive a separate file permission tool and does not need a
failed permission-request turn. Protected paths and explicit deny rules remain
blocked without a prompt. Saved rights live in
`~/.pi/agent/io-permissions.json`, keyed by the real workspace path.

Project `.pi` is also read-only by default because it can load code and prompts
on reload. A write asks for the whole project `.pi` folder, like repository
control writes ask for `.git`. Symlinked `.git` and `.pi` control folders cannot
receive write grants because their targets could be much broader than the path
shown in the prompt. Global `~/.pi` stays blocked and cannot receive a model
grant.

A Codex-backed bash or background-job start can declare one exact
`network_host` right in its `permissions`. The request rejects schemes, ports,
paths, and wildcards. An allow-once host stays in that command's generated
profile and cannot move to a parallel call. The separate
`request_network_permission` tool saves an exact host for the workspace; it no
longer creates a shared next-command grant. Old blanket `web` and
`local_network` rights are ignored when saved rights load.

A bash or background-job start can declare up to 16 exact read, write, or
network host rights. The sandbox checks those rights and asks before launch. An allow-once choice is
kept in that one tool call's generated profile, so another command cannot use
it. This handles tools that hide the OS access error, such as a stateful CLI
that reports only that its service is unavailable.

On the Codex backend, undeclared bash rights still use a limited fallback. The
sandbox checks failed command output for access errors such as `Operation not
permitted` and `Permission denied`. If the
same error line has
one exact absolute path and the active policy identifies it as a write denial,
the sandbox shows the same approval prompt and retries inside the original tool
call. It allows at most three retries. A retry can repeat work completed before
the denied operation, so the prompt says that bash will retry. Protected paths
and explicit deny rules never prompt. Failures without a safe exact path return
unchanged as regular command failures. Network failures remain blocked until
the model requests the exact host or IP.

Workspace write access includes creating, changing, and deleting workspace
files. The sandbox does not inspect command text, so it does not add a second
prompt for a delete command. Codex starts with repository control data such as
`.git` read-only. If Git fails on a file under `.git`, the approval prompt asks
for the repository's whole `.git` folder and retries the command. Use version
control or backups for other workspace files.

Enabling an MCP service also needs user approval because an MCP server can read
private data or act outside the shell sandbox. That approval is scoped to the
named service and can be saved for one workspace.

Unix socket access grants a right to another service. That service may do work
outside the caller's file or network limits. The machine config allows the Nix
daemon for normal Nix, flake, and direnv work. The reserved background-job tmux
socket is never passed to normal bash even if an older config lists it.

On the Codex backend, use the `background_job` tool for dev servers, watchers,
builds, and long tests. The tool owns the tmux socket and starts each command in
a fresh Codex sandbox with the current workspace rights. It can list, inspect,
send input to, and stop only jobs marked as broker-managed. Native preview
background starts fail closed until they use the same broker policy.

Pi's built-in recursive `grep` and `find` tools do not run as child processes
of the sandbox. The extension removes them from the active tool set, and still
blocks calls if another extension turns them back on. Agents use `rg` or `fd`
through bash, where Codex enforces the same file policy.

Global sandbox config lives at `~/.pi/agent/extensions/sandbox.json`. A trusted
project can add tighter rules at `.pi/sandbox.json`; project config cannot add
rights or turn the sandbox off. Static `network.allowedDomains` entries must
also be exact hostnames or IPs. Wildcards and broad local-network switches are
not accepted.

`shellEnvironment` follows Codex's shell policy order. Its `inherit` value is
`all`, `core`, or `none`; the default is `core`. Unless
`ignoreDefaultExcludes` is true, variable names containing `KEY`, `SECRET`, or
`TOKEN` are removed. `exclude` then removes case-insensitive glob matches,
`set` adds host-configured values, and `includeOnly` applies a final allowlist.
Only the global file may add values with `set`. A trusted project may choose a
stricter inheritance mode, add excludes, or add an allowlist.
