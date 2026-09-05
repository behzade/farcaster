# Modal keyboard implementation plan

Status: proposed; composer layout implemented separately. No modal bindings are
active yet.

## Interaction contract

Farcaster owns a chat normal mode, not a global Vim mode. Embedded Neovim and
terminal retain their own input semantics. Clicking an incidental control must
not silently change the primary keyboard owner.

| Chat normal keys | Action |
| --- | --- |
| `j` / `k` | Move transcript cursor down / up, revealing it as needed |
| `i` | Focus composer and enter insert mode |
| `v` | Start transcript visual selection |
| `/` | Open session search (not transcript search) |
| `Space e` | Show/focus embedded editor |
| `Space t` | Show/focus terminal |
| `Space j` / `Space k` | Next / previous session |
| `Space 1`–`Space 9` | Jump using existing visible-session numbering |
| `Space 0` | Existing first-unsubmitted-draft action |
| `Escape` | Cancel pending leader or visual selection |

Space in composer, search fields, terminal, or Neovim remains literal input.
Do not reuse the existing modifier suffixes blindly: `t` currently creates a
session and `j` currently opens terminal. New-session and other leader commands
need explicit assignments before release.

## 1. Explicit ownership and restoration

Implement a backend-neutral state/transition module under `src/app/ui/` with
primary owners Transcript, Composer, Editor, Terminal; chat modes Normal,
Insert, Visual; and explicit pending-leader state. Keep transcript mode/cursor
per session; never carry a partial leader sequence across sessions.

Integrate through:

- `src/app/mod.rs`, `src/app/bootstrap.rs`: state and focus handles.
- `src/app/workspace/surfaces.rs`: active-surface transitions, post-render focus,
  existing dialog/sheet return-focus mechanisms.
- `src/app/views/root/actions.rs`: commands use transitions rather than ad hoc
  focus calls.
- `src/app/ui/primitives/` and composer runtime controls: pointer activation of
  buttons/toggles preserves the prior primary owner. Dropdowns capture keys
  only while open. Explicit Tab navigation still reaches controls.

Temporary UI needs a return target containing owner, chat mode, and focus
handle. Nested dialogs restore in order; if a target was removed, fall back to
the active chat transcript rather than a stale handle. Do not globally force
focus every render, which would break text fields and accessible navigation.

Tests: pointer toggle then `j`; dropdown select/cancel then `j`; nested dialogs;
Tab/Shift-Tab traversal; session deletion during an overlay; surface switching.

## 2. Normal/insert modes and Escape policy

Add a transcript focus context distinct from composer/input and embedded
surfaces. Mode must not be inferred from “composer lacks focus.” `i` restores
the existing draft/caret; leaving insert mode preserves its contents.

Proposed modal Escape precedence:

1. Dismiss the active completion/menu/dialog.
2. Otherwise leave composer insert mode for transcript normal.
3. In visual mode, clear selection and return to normal.
4. In normal mode, cancel pending leader; otherwise no destructive action.

This intentionally conflicts with today's Escape-to-apply-steer/double-Escape-
to-abort behavior in `src/app/composer/submissions.rs`. Before enabling modality,
assign explicit apply-steer/abort commands and update help/tests. Do not allow
one Escape both to exit insert mode and arm an abort. Preserve legacy behavior
only in the legacy keymap profile.

Render `NORMAL · TRANSCRIPT`, `INSERT · COMPOSER`, or `VISUAL · …` on the left
of the new status strip in `src/app/views/composer/footer/mod.rs`. Usage remains
on the right. Do not display invented mode state before ownership is wired up.
Embedded surfaces show their own mode indicators.

## 3. Space leader and session search

Extend `src/app/ui/keybindings.rs` with a modal profile, using GPUI chord support
where its cancellation/input routing meets the contract. Otherwise use the
small explicit pending-leader state, not a second independent command system.
All actions route through existing backend-neutral app commands.

A pending Space shows available next keys in a temporary hint surface without
taking text focus. Escape, an unknown continuation, focus loss, surface/session
change, or opening a dialog clears it. Unknown continuations are consumed in
normal mode, never replayed into a newly focused composer or shell. Key repeat
must not repeatedly open surfaces.

Connect `/` to the existing session picker via `src/app/navigation/picker.rs`;
it receives text normally. Dismissal restores normal mode; selection activates
the target session in normal mode. Help, session badges, and workspace labels
must derive from the active registry rather than the current modifier helper.

Tests: each mapping, pending-leader cancellation, numbering/order parity,
search typing, no capture of spaces/slashes/j/k in text or embedded inputs.

## 4. Transcript cursor, then visual selection

First inspect and extend the existing virtualized list/selection machinery in
`src/app/views/transcript/list.rs`, `src/app/views/transcript/list/`, and
`src/app/ui/keyboard.rs`. Reuse clipboard serialization and existing mouse
selection behavior; do not introduce a disconnected keyboard-only selection.

Recommended first contract: `j/k` moves by selectable transcript block, and
`v` selects an inclusive block range. Give the active block a visible cursor
marker and label visual selection in blocks, not lines. This needs product
confirmation: rendered-line navigation is a materially larger follow-up and
must not be implied by the first release.

Use stable message/block identities rather than raw virtualized row indices.
Reveal the active cursor, pause tail-follow on intentional navigation, preserve
anchors during streaming/reflow, and recover deterministically when a block
collapses/disappears. `y` copies the selected range and returns to normal.
Provide an explicit return-to-live-tail command.

Tests: code/tool blocks, cross-message selection, virtualization gaps, streaming
appends, resize/reflow, collapse/expand, clipboard content, empty transcript,
per-session cursor restoration, and existing mouse-selection regressions.

## 5. Embedded boundary and rollout

Unresolved release decision: a configurable escape-to-app chord is required
from editor/terminal. Prototype `Ctrl+g` as an opt-in candidate, not a silently
reserved default; it conflicts with Neovim file status, shell commands, and
Zellij. Supply an explicit way to send that literal chord to the embedded app.
Do not intercept ordinary Escape or Space inside either embedded surface.

Keep transport/input implementation under the existing editor/terminal
boundaries (`src/app/workspace/editor.rs`, `terminal.rs` and their integrations).
No Pi-specific transport or session semantics belong in modal state.

Land ownership first, then normal/insert and leader navigation behind an opt-in
profile; add visual selection after its cursor contract passes tests. Preserve
the legacy modifier profile until the embedded escape and abort/steer decisions
are settled. Update `docs/usage.md` and shortcut help with the shipped profile.

Validation: narrow unit tests for transitions/keymap/list first, then integrated
focus routing tests and `cargo check --bin farcaster`; always `git diff --check`.
Manually exercise Linux desktop/input-method conflicts, embedded Neovim and
shell passthrough, pointer controls, keyboard-only dialogs, and narrow windows.
