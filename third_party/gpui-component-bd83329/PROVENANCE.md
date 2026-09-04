# GPUI Component subset provenance

This directory contains an application-specific source extraction from
[Longbridge GPUI Component](https://github.com/longbridge/gpui-component) at
commit `bd833291311289f3468479d31b629d3de279d3d4` (upstream version 0.5.2).
The extracted code is distributed under Apache-2.0; the exact upstream license
is preserved in `LICENSE-APACHE`.

## Included source

`base/src` retains the infrastructure required by Pi's input, text-selection,
popup, tooltip, list, and positioning behavior. The upstream input engine is
kept complete to preserve editing, selection, IME, clipboard, undo, search,
and accessibility behavior.

`ui/src` retains the implementations required by Pi's buttons, inputs,
textareas, transcript text and Markdown rendering, lists, popup menus,
tooltips, themes, and root overlays. `ui/locales/ui.yml` is copied unchanged
for messages used by those components.

Files and modules unrelated to this runtime closure were omitted. Examples
include charts, calendars, docks, tables, settings, sidebars, ratings, and
standalone application-menu components.

## Local adaptations

The component implementations are preserved except for narrow dependency-glue
changes:

- `base/src/lib.rs` and `ui/src/lib.rs` expose and initialize only the retained
  module closure.
- `ui/src/root.rs` retains content rendering, selection, input tracking,
  tooltips, native input menus, focus navigation, and window borders. Upstream
  sheet, dialog, and notification facilities are omitted because Pi owns those
  surfaces and never calls the component APIs.
- `ui/src/button/mod.rs`, `ui/src/input/mod.rs`, `ui/src/menu/mod.rs`, and
  `ui/src/highlighter/mod.rs` omit component variants Pi does not build.
- `base/src/number_input.rs` retains only numeric-step types required by the
  shared input engine; `base/src/input/mod.rs` keeps their public re-exports.
- `ui/src/button/button_icon.rs` omits the unused progress-circle variant.
- `ui/src/input/state.rs` omits the unused OTP state, while retaining input,
  textarea, and editor state behavior.
- `ui/src/menu/popup_menu.rs` adds `with_selected_index` so a nested menu can open on the current item.
- `ui/src/select.rs` retains only the caret used by dropdown buttons.
- `ui/src/theme/mod.rs` omits settings for removed sheet and notification
  modules. `ui/src/text/node.rs` always uses the retained non-tree-sitter
  highlighter path.
- `ui/src/native_menu/mod.rs` omits upstream tests tied to the removed Lucide
  asset source.
- `ui/src/icon.rs` replaces the generated upstream icon enum and asset build
  pipeline with a hand-written enum mapped to Pi's bundled Phosphor icons.
- The crate manifests use Pi's vendored GPUI revision, remove dependencies of
  omitted modules, disable unused default AppKit features, and disable the
  incomplete upstream crate-local test suites. Pi's application tests remain
  the behavioral verification boundary.

The upstream `gpui-component-assets` and `gpui-component-macros` crates and the
Lucide asset bundle are not included.
