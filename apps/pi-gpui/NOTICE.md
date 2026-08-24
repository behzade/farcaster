# Notices

Pi GPUI is licensed under GPL-3.0-or-later. See `LICENSE`.

Portions of the native presentation structure, theme adapter, button/dialog
primitives, responsive layout policy, focus handling, and off-thread update
patterns were adapted from the local Issues project at:

- Source: `/mnt/fast/Projects/issues`
- Commit: `2df4b944983889305e4e196408b400d06f571bfd`
- Upstream license: GPL-3.0-or-later

Those portions were modified for a Pi RPC client: Issues product state,
database, source-control, issue, and review behavior were removed; the visual
system was changed to Pi's Gruvbox dark-hard palette; and the application model
was replaced with a project-scoped Pi subprocess and transcript.

The Pi RPC protocol is consumed as a public subprocess interface. No Pi source
or extension code is included or modified by this module.

Lilex font binaries were copied from Zed commit
`ce6f3af5f7ae2bbdb002c8ce5cc38e96179de811`, which uses Lilex for its
`.ZedMono` alias. Lilex is copyright Mikhael Khrustik and contributors, based
on IBM Plex Mono, and is distributed under the SIL Open Font License 1.1. The
font license is included at `assets/lilex/OFL.txt`; the font files remain under
that license. Upstream: <https://github.com/mishamyrt/Lilex>.

Vazirmatn Regular, Medium, SemiBold, and Bold v33.003 are bundled as the
Persian/Arabic UI font.
Vazirmatn is copyright the Vazirmatn Project Authors and is distributed under
the SIL Open Font License 1.1. Its license is included at
`assets/vazirmatn/OFL.txt`; the font files remain under that license. Upstream:
<https://github.com/rastikerdar/vazirmatn>.

The renderer-neutral diff planning structure, patch parsing behavior, split-row
alignment, and intraline-span behavior in `src/diff_plan` were adapted from
`@pierre/diffs` by Pierre Computer Company at commit
`55a941914056af44c78c4ba607b37130f189fb70`. They were rewritten in Rust to
produce immutable native render plans instead of HAST, CSS, or DOM output.
Pierre Diffs is distributed under Apache-2.0; its license is included at
`THIRD_PARTY_LICENSES/PIERRE_DIFFS_APACHE-2.0.txt`. Upstream:
<https://github.com/pierrecomputer/pierre/tree/55a941914056af44c78c4ba607b37130f189fb70/packages/diffs>.

The direct Longbridge GPUI Component dependencies `gpui-component`,
`gpui-component-assets`, and `gpui-fps` are pinned to commit
`bd833291311289f3468479d31b629d3de279d3d4` and distributed under Apache-2.0.
The `gpui-component-assets` crate embeds Longbridge assets at runtime. The exact
upstream `LICENSE-APACHE` is included at
`THIRD_PARTY_LICENSES/GPUI_COMPONENT_ASSETS_APACHE-2.0.txt`.

The GPUI framework and its narrow Zed package closure are included from Zed
commit `cc053a4a6fa2fd0e8793201ed9099466af1be0b1` under
`third_party/zed-gpui-cc053a4`. The included packages declare Apache-2.0 or
GPL-3.0-or-later licensing. Exact upstream license texts and detailed source
provenance are included in that directory as `LICENSE-APACHE`, `LICENSE-GPL`,
and `README.md`.

The locally patched `gpui-neovim` dependency derives from release `0.1.0`, and
`gpui-libghostty` uses release `0.1.2`, from
<https://github.com/behzade/gpui-libghostty>. The patched Neovim source is under
`third_party/gpui-neovim-0.1.0`; it and the pinned Ghostty source are distributed
under MIT, with exact licenses and provenance retained in that repository.

Application icons copied from Phosphor Icons are distributed under MIT. The
exact upstream license is included at
`THIRD_PARTY_LICENSES/PHOSPHOR-ICONS-MIT.txt`.

The fallback embedded SVG icon set identifies itself as Lucide and includes work
derived from Feather. Its ISC and Feather MIT terms and notices are included at
`THIRD_PARTY_LICENSES/LUCIDE-ISC.txt`; `gpui-component-assets::Assets` embeds
and serves that set at runtime.
