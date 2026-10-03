# Notices

Farcaster is licensed under GPL-3.0-or-later. Release bundles include `LICENSE`,
this notice, and the relevant bundled-component texts under
`THIRD_PARTY_LICENSES`.

The complete corresponding source for each release, including build scripts and
vendored modifications, is available from the matching tag at
<https://github.com/behzade/farcaster/releases>.

Portions of the native presentation structure, theme adapter, button/dialog
primitives, responsive layout policy, focus handling, and off-thread update
patterns were adapted from the local Issues project at:

- Source: `/mnt/fast/Projects/issues`
- Commit: `2df4b944983889305e4e196408b400d06f571bfd`
- Upstream license: GPL-3.0-or-later

IBM Plex Sans font binaries were copied from IBM Plex commit
`bf260093582f04622aacc1e9f9ca604d7ccd0c42`. IBM Plex is copyright IBM Corp.
and is distributed under the SIL Open Font License 1.1. Its license is included at
`THIRD_PARTY_LICENSES/IBM-PLEX-OFL.txt`. Upstream:
<https://github.com/IBM/plex>.

Lilex font binaries were copied from Zed commit
`ce6f3af5f7ae2bbdb002c8ce5cc38e96179de811`. Lilex is copyright Mikhael
Khrustik and contributors, based on IBM Plex Mono, and is distributed under
the SIL Open Font License 1.1. Its license is included at
`THIRD_PARTY_LICENSES/LILEX-OFL.txt`. Upstream:
<https://github.com/mishamyrt/Lilex>.

Vazirmatn Regular, Medium, SemiBold, and Bold v33.003 are bundled as the
Persian/Arabic UI font.
Vazirmatn is copyright the Vazirmatn Project Authors and is distributed under
the SIL Open Font License 1.1. Its license is included at
`THIRD_PARTY_LICENSES/VAZIRMATN-OFL.txt`. Upstream:
<https://github.com/rastikerdar/vazirmatn>.

The application-specific `gpui-base` and `gpui-component` source subset is
extracted from Longbridge GPUI Component commit
`bd833291311289f3468479d31b629d3de279d3d4` and distributed under Apache-2.0.
The upstream license is included at
`THIRD_PARTY_LICENSES/GPUI-COMPONENT-APACHE-2.0.txt`; extraction details remain
with the corresponding source under `third_party/gpui-component-bd83329`.

The application uses the crates.io `gpui-pre` 0.3.7 snapshot and related
packages, built from Zed commit `1a28cff4b409169bac058bca40dfbfeb7621d19b`.
They declare Apache-2.0 and retain Copyright 2022 - 2025 Zed Industries, Inc.
The Apache-2.0 license text is included at
`THIRD_PARTY_LICENSES/GPUI-COMPONENT-APACHE-2.0.txt`.

The `gpui-libghostty` dependency uses crates.io release `0.3.1` from
<https://github.com/behzade/gpui-libghostty>. It and the pinned Ghostty source
are distributed under MIT. The local Neovim transport in
`src/app/workspace/neovim.rs` is derived from that project's `gpui-neovim` 0.1.6.
Their licenses are included at
`THIRD_PARTY_LICENSES/GPUI-LIBGHOSTTY-MIT.txt` and
`THIRD_PARTY_LICENSES/GHOSTTY-MIT.txt`; detailed provenance remains in the
upstream source.

Application icons copied from Phosphor Icons are distributed under MIT. The
exact upstream license is included at
`THIRD_PARTY_LICENSES/PHOSPHOR-ICONS-MIT.txt`.

The Pi application icon is adapted from Pi's official website assets, and the
OpenCode application icon is adapted from OpenCode's official brand assets.
Both are distributed under MIT. Their exact upstream licenses are included at
`THIRD_PARTY_LICENSES/PI-WEBSITE-MIT.txt` and
`THIRD_PARTY_LICENSES/OPENCODE-MIT.txt`.

The Antigravity, Claude, Codex, and Cursor application icons are adapted from
Lobe Icons and distributed under MIT. Their upstream source is
<https://github.com/lobehub/lobe-icons>, and the exact license is included at
`THIRD_PARTY_LICENSES/LOBE-ICONS-MIT.txt`.

Linux AppImages explicitly bundle libxcb, Wayland EGL, the Vulkan
loader, and libglvnd's libEGL and libGLdispatch. Their license and attribution
texts are included under `THIRD_PARTY_LICENSES`.

File-type icons under `assets/file-icons` are from Material Icon Theme,
distributed under MIT, and retain their upstream colors. The exact upstream
license is included at `THIRD_PARTY_LICENSES/MATERIAL-ICON-THEME-MIT.txt`.
Pinned revision and asset mappings are documented in `assets/file-icons/README.md`.
Upstream: <https://github.com/material-extensions/vscode-material-icon-theme>.

The GNU Emacs editor icon comes from Simple Icons under CC0 1.0 Universal.
Upstream: <https://github.com/simple-icons/simple-icons>. See
`THIRD_PARTY_LICENSES/SIMPLE-ICONS-CC0-1.0.md`.

The GNU nano editor icon comes from file-icons under the ISC license.
Upstream: <https://github.com/file-icons/icons>. See
`THIRD_PARTY_LICENSES/FILE-ICONS-ISC.txt`.

The Micro editor icon is adapted from Micro's logo under the MIT license.
Upstream: <https://github.com/zyedidia/micro>. See
`THIRD_PARTY_LICENSES/MICRO-MIT.txt`.
