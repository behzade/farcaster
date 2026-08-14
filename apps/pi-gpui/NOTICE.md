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

The pinned `gpui-component-assets` dependency embeds Longbridge GPUI Component
assets at runtime. Longbridge distributes that crate under Apache-2.0; the
exact `LICENSE-APACHE` from pinned commit
`bc174a7ec4534b2a4174fddde314b38d30d69093` is included at
`THIRD_PARTY_LICENSES/GPUI_COMPONENT_ASSETS_APACHE-2.0.txt`.

The embedded SVG icon set identifies itself as Lucide and includes work derived
from Feather. Its ISC and Feather MIT terms and notices are included at
`THIRD_PARTY_LICENSES/LUCIDE-ISC.txt`. No SVG was copied into this module's
source tree, but `gpui-component-assets::Assets` embeds and serves that set at
runtime.
