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
`PATH`. The root `nix develop .#pi-gpui` shell supplies the native GPUI build
dependencies without rebuilding the Home Manager configuration.

## Check

```sh
CARGO_TARGET_DIR="$PWD/target" nix develop .#pi-gpui -c cargo fmt --manifest-path apps/pi-gpui/Cargo.toml --check
CARGO_TARGET_DIR="$PWD/target" nix develop .#pi-gpui -c cargo test --manifest-path apps/pi-gpui/Cargo.toml
CARGO_TARGET_DIR="$PWD/target" nix develop .#pi-gpui -c cargo check --manifest-path apps/pi-gpui/Cargo.toml
CARGO_TARGET_DIR="$PWD/target" nix develop .#pi-gpui -c cargo clippy --manifest-path apps/pi-gpui/Cargo.toml --all-targets -- -D warnings
```

See `NOTICE.md` for adapted-code attribution.
