# Pi GPUI

A native GPUI client for the installed Pi coding agent. This module is a
separate GPL-3.0-or-later work inside the enclosing MIT repository.

Pi GPUI starts one `pi --mode rpc` process for the active root session. It does
not replace or modify Pi: normal settings, context files, extensions, skills,
prompt templates, authentication, and sandbox behavior load in the selected
project directory.

Before opening a project with local Pi resources, the app honors saved trust
and `defaultProjectTrust`, or asks for a project, parent, or deny decision.
`/trust` opens the same persistent trust manager. Changes made for an
already-running project take effect after restarting Pi GPUI.

Session details separates the repository's authoritative **Working copy** from
Pi's recorded **Session activity**. Working copy is read-only: it shows Git's
staged, working-tree, untracked, and conflict state, or Jujutsu's current
change, and opens backend-specific file diffs. Choose Auto, Git, or Jujutsu per
project; Auto uses Jujutsu for a colocated repository. A Jujutsu refresh may
snapshot the working copy as part of normal `jj` behavior. Repository commands
remain disabled for explicitly untrusted projects. Set `PI_GUI_GIT` or
`PI_GUI_JJ` to override either executable.

Changed files in Session details can be opened in an embedded Neovim editor.
The editor uses Ghostty's VT core rendered directly by GPUI and keeps one
Neovim process per open editor surface. Set `PI_GUI_NVIM` to override the
`nvim` executable.

## Run

```sh
make run
make run PROJECT=/path/to/project
```

The project defaults to the repository directory. `pi` must be available on
`PATH`. The root `.envrc` supplies Cargo and the native GPUI build environment.
Use those tools directly. Do not run a Nix command unless the user asks for that
exact check.

After the first prompt creates a session, the GPUI companion extension creates
a semantic session title with Pi's configured model registry. It uses
`openai-codex/gpt-5.6-luna` by default; set `PI_GUI_TITLE_MODEL` to any fully
qualified Pi model selector to use another lightweight model. Existing and
manually edited session names are never replaced automatically.

## Check

```sh
make check-gpui
```

For a focused check, invoke Cargo directly and keep build output in the shared
repository target:

```sh
cargo test --manifest-path apps/pi-gpui/Cargo.toml
```

## Transcript benchmark

Run the mock streaming benchmark in release mode:

```sh
cargo bench --manifest-path apps/pi-gpui/Cargo.toml --bench transcript
```

It reports median, p95, and maximum time for event reduction, row projection and
list synchronization, GPUI drawing, and the complete frame at 200, 2,000, and
10,000 historical transcript items.

See `NOTICE.md` for adapted-code attribution.
