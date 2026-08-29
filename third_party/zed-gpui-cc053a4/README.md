# Narrow Zed GPUI source snapshot

This directory contains the Cargo package/source closure used by Pi GPUI from
<https://github.com/zed-industries/zed> commit
`cc053a4a6fa2fd0e8793201ed9099466af1be0b1`. This is the Zed revision pinned by
gpui-component commit `bd833291311289f3468479d31b629d3de279d3d4`.

Included packages are `collections`, `derive_refineable`, `gpui`, `gpui_linux`,
`gpui_macos`, `gpui_macros`, `gpui_platform`, `gpui_shared_string`, `gpui_util`,
`gpui_web`, `gpui_wgpu`, `gpui_windows`, `http_client`, `media`, `perf`,
`refineable`, `scheduler`, `sum_tree`,
`util_macros`, `zlog`, `ztracing`, and `ztracing_macro`. This is the complete
Zed package closure selected by `apps/pi-gpui/Cargo.lock`; dependencies hosted
in other upstream repositories remain ordinary pinned Cargo dependencies.

The snapshot was exported from that commit without Git metadata, Zed
application code or assets, build outputs, unrelated workspace members, or
test/example directories and explicit example/bench targets. Test modules
that are inline or part of retained source files remain. The root workspace
member list was narrowed to the included packages. Its dependency and profile
catalogs were retained because the included package manifests use Cargo
workspace inheritance, even though many app packages referenced by catalog
entries are not present.

For GPL auditability, the narrowing modifications were made on 2026-08-16 to
these upstream manifests:

- `Cargo.toml` (workspace member and default-member narrowing)
- `crates/gpui/Cargo.toml` (dev dependencies and example targets omitted)
- `crates/gpui_macos/Cargo.toml` (dev dependency omitted)
- `crates/gpui_macros/Cargo.toml` (dev dependency omitted)
- `crates/gpui_wgpu/Cargo.toml` (dev dependencies and bench target omitted)
- `crates/sum_tree/Cargo.toml` (dev dependencies omitted)
- `crates/zlog/Cargo.toml` (dev dependency omitted)

Runtime source and runtime feature definitions are otherwise unchanged.

The included packages declare Apache-2.0 or GPL-3.0-or-later licensing. Exact
copies of the upstream commit's `LICENSE-APACHE` and `LICENSE-GPL` are included
at this directory's root; package-local license files are regular-file copies of
those texts so the snapshot contains no symlinks.
