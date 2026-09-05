# Development

[← README](../README.md)

```sh
make check
cargo bench --bench transcript
```

Run a release build against the local `gpui-libghostty` and `gpui-neovim`
crates in `../gpui-ghostty`:

```sh
make release-local
# Alternate checkout or project to open:
make release-local GPUI_GHOSTTY_DIR=/path/to/gpui-ghostty PROJECT=/path/to/project
```

This uses invocation-only Cargo path overrides and the existing target directory;
`Cargo.toml` and `Cargo.lock` stay unchanged. Normal `make release` still uses the
published crates. Cargo may warn if the local crates' dependency lists differ
from the published versions; path overrides are intended for testing code changes,
not dependency-graph changes.

Create a native bundle with Cargo Packager 0.11.8:

```sh
cargo install cargo-packager --version 0.11.8 --locked
make bundle
```
