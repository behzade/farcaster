# Notices

Farcaster is licensed under GPL-3.0-or-later. See `LICENSE`.

Portions of the native presentation structure, theme adapter, button/dialog
primitives, responsive layout policy, focus handling, and off-thread update
patterns were adapted from the local Issues project at:

- Source: `/mnt/fast/Projects/issues`
- Commit: `2df4b944983889305e4e196408b400d06f571bfd`
- Upstream license: GPL-3.0-or-later

Those portions were modified for a coding-agent client: Issues product state,
database, source-control, issue, and review behavior were removed; the visual
system was changed to a Gruvbox dark-hard palette; and the application model
was replaced with project-scoped agent sessions and transcripts.

The current Pi backend consumes Pi's public RPC subprocess interface. No Pi
source or extension code is included or modified by Farcaster.

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

The application-specific `gpui-base` and `gpui-component` source subset is
extracted from Longbridge GPUI Component commit
`bd833291311289f3468479d31b629d3de279d3d4` and distributed under Apache-2.0.
The exact upstream license and extraction details are included under
`third_party/gpui-component-bd83329` as `LICENSE-APACHE` and `PROVENANCE.md`.

The GPUI framework and its narrow Zed package closure are included from Zed
commit `cc053a4a6fa2fd0e8793201ed9099466af1be0b1` under
`third_party/zed-gpui-cc053a4`. The included packages declare Apache-2.0 or
GPL-3.0-or-later licensing. Exact upstream license texts and detailed source
provenance are included in that directory as `LICENSE-APACHE`, `LICENSE-GPL`,
and `README.md`.

The `gpui-neovim` and `gpui-libghostty` dependencies use crates.io releases
`0.1.1` and `0.1.4`, respectively, from
<https://github.com/behzade/gpui-libghostty>. They and the pinned Ghostty source
are distributed under MIT, with exact licenses and provenance retained in that
repository.

Application icons copied from Phosphor Icons are distributed under MIT. The
exact upstream license is included at
`THIRD_PARTY_LICENSES/PHOSPHOR-ICONS-MIT.txt`.
