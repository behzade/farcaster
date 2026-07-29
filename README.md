# Pi

This repo owns Behzad's reviewed Pi coding-agent setup. It contains local
extensions, Codex and native sandbox code, background job support, tests, and
pinned Nix builds.

Machine policy stays in `nix-config`. That repo sets sandbox paths, network
hosts, and notification settings. This repo owns code and package pins.

## Layout

- `extensions/sandbox`: fail-closed sandbox adapter, IO permission prompt,
  native broker client, and background-job tool and helper.
- `nix/pi-mcp-cli.nix`: pinned stateless MCP CLI available inside sandboxed
  shell commands.
- `nix/pi-web-access.nix`: pinned `pi-web-access` package providing web search,
  content extraction, and video understanding tools.
- `sandbox-broker`: protocol, threat model, provenance, and the first native
  macOS Seatbelt backend.
- `extensions/*.ts`: local notification, input, session, title, and dense tool
  rendering hooks.
- `themes/gruvbox-dark-hard.json`: Gruvbox's canonical dark-hard palette for
  Pi. The palette values come from
  [morhetz/gruvbox](https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim).
  Its token roles follow Zenbones' contrast-first hierarchy: neutral lightness
  carries most structure, while hue marks links, active UI, state, and diffs.
- `nix`: the complete Pi agent package plus pinned component builds.
- `tests`: shared policy and notification checks.

## Checks

```sh
npm run check --prefix extensions/sandbox
cargo test --manifest-path sandbox-broker/Cargo.toml
node --test tests/governance.test.ts
nix flake check
```

Build the complete deployable agent tree with:

```sh
nix build
```

Individual components remain available for focused development:

```sh
nix build .#sandbox
nix build .#sandbox-broker
nix build .#dense-tools
nix build .#mcp-cli
nix build .#subagents
nix build .#openai-server-compaction
nix build .#web-access
```

`nix-config` consumes this repo's default package as a flake input and deploys
that tree recursively at `~/.pi/agent`. The package owns Pi's prompt, extension,
skill, and theme inventory; machine-specific policy files remain in
`nix-config`.

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

The tree also contains opt-in native backends for macOS Seatbelt and Linux
Bubblewrap. The macOS integration and denial-collector gate passes. Its cleanup
uses a process group plus a best-effort kqueue descendant tracker with process
start-time checks. Deliberate fast `setpgid`, `setsid`, or double-fork escape is
out of scope because macOS has no unprivileged process-tree owner; a survivor
still keeps its command's Seatbelt limits.

The Linux broker uses a Nix-pinned Bubblewrap path, a read-only host root, exact
write mounts, protected child mounts, user/PID/network namespaces, and a
reviewed blocked-network seccomp filter. It reports ready only after a real
namespace, private `/proc`, seccomp, and `NoNewPrivs` self-test. Linux remains on
Codex in machine config until its ignored release gate runs on both x86_64 and
aarch64 Linux.

The extension package pins the matching broker store path on both systems. A
custom `brokerPath` must be absolute and can come only from global config;
project config cannot switch backends or replace the broker. Protocol v1 keeps
network and Unix sockets blocked and has no native background jobs. On macOS,
one bounded session collector returns incomplete structured Seatbelt denial
hints; Linux emits no denial hints. `backend: "codex"` remains the default. To
opt in on a supported host, set this in the global
`~/.pi/agent/extensions/sandbox.json` file:

```json
{
  "backend": "native-preview"
}
```

The default rights are:

- read most of the system;
- deny reads and writes for `.env` and `.key` files and for `.ssh`, `.aws`,
  and `.gnupg`; deny reads of Pi and Codex `auth.json`; keep PEM certificate
  bundles readable but read-only;
- keep `~/.pi` and `~/.codex` read-only so a grant cannot change its own
  policy;
- write the workspace, `/tmp`, the system temp folder, and narrow development
  cache roots used by Cargo, Go, npm, pnpm, Bun, Yarn, Corepack, Deno, pip, and
  uv; package-manager config, credential files, and global install bins stay
  outside these implicit write rights;
- pass only core shell variables such as `PATH`, `HOME`, locale, and temp paths;
- remove variables whose names contain `KEY`, `SECRET`, or `TOKEN`;
- block every network host until the user grants that exact hostname or IP;
- block local and private network targets.

Pi creates missing fixed cache directories from the trusted host before the
sandbox starts. Cache rights are typed as files or folders and omitted when the
root or an ancestor below the home folder is a symlink. They are shared
across workspaces, so a hostile project can still poison mutable cache state for
a later build; use separate users or disposable homes when that risk matters.
These rights support local builds and already-cached dependencies. Native
protocol v1 still cannot fetch missing dependencies because network grants have
not landed.

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

On the native macOS backend, a failed command can return best-effort kernel
denial hints even when the app hides `EPERM`. Pi prompts and retries only when
a hint names one exact policy-safe file or folder path. When four distinct file
hints share one parent folder and access kind, Pi shows one prompt with choices
to grant the exact files or their parent folder recursively, once or always for
the workspace. It never widens access without the user's choice and stops after
eight total attempts. A broad folder choice remains subject to hard protected
paths and configured denies. Empty,
late, malformed, protected, denied, or `/dev` device hints grant nothing.
Unified logging can miss denials, so declared rights remain the reliable path.

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
prompt for a delete command. Repository control data under the active workspace,
including nested repositories, starts read-only. If Git fails on a file under
`.git`, the approval prompt asks for that repository's whole `.git` folder and
retries the command. `.git` data under configured package caches or temp paths
is normal cache data and does not trigger a repository-control prompt. Use
version control or backups for other workspace files.

The sandbox package puts the pinned `mcp-cli` binary on the command environment's
`PATH`. Its wrapper fixes `MCP_NO_DAEMON=1`, so every invocation connects,
initializes, performs one discovery or tool call, closes, and exits inside that
command's sandbox. MCP has no Pi extension, dynamically registered tools, approval
service, background client, or persistent process. Remote MCP access uses the same
exact `network_host` declaration and approval as any other shell command.

Machine-specific MCP server configuration belongs in `nix-config`/Home Manager,
alongside the sandbox host allowlist. Set its absolute path as
`shellEnvironment.set.MCP_CONFIG_PATH` in the global sandbox config; this also
prevents a repository-local `mcp_servers.json` from taking precedence. The model
discovers and invokes configured tools with `mcp-cli grep`, `mcp-cli info`, and
`mcp-cli call`.

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
