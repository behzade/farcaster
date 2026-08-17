# Pi GPUI

A native GPUI client for the installed Pi coding agent. This module is a
separate GPL-3.0-or-later work inside the enclosing MIT repository.

Pi GPUI starts one `pi --mode rpc` process for the active root session. It does
not replace or modify Pi: normal settings, context files, extensions, skills,
prompt templates, authentication, and sandbox behavior load in the selected
project directory.

## Run

```sh
make run
make run PROJECT=/path/to/project
```

The project defaults to the repository directory. `pi` must be available on
`PATH`. The root `.envrc` supplies Cargo and the native GPUI build environment.
Use those tools directly. Do not run a Nix command unless the user asks for that
exact check.

## Check

```sh
make check-gpui
```

For a focused check, invoke Cargo directly and keep build output in the shared
repository target:

```sh
cargo test --manifest-path apps/pi-gpui/Cargo.toml
```

See `NOTICE.md` for adapted-code attribution.
