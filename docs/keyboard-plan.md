# Modal keyboard implementation plan

Status: navigation implemented with revised one-shot activation. `Ctrl+g`
(plus `Cmd+g` on macOS) activates app keys for one second without moving focus;
double chord returns to chat normal; `i`/`a`, bare `0`–`9`, `/`, and Space-prefixed workspace
navigation are active. Composer status shows the current focus mode and pending
leader hints. Existing direct shortcuts remain during this incremental rollout.

Transcript scrolling is active: `j/k` takes small steps, `Ctrl+f/b` pages,
and `Ctrl+d/u` half-pages. Key repeat is supported, and scrolling uses the
existing batched list path, including pause/resume of live-tail following.

Pointer-focus preservation now covers app-owned action rows as well as buttons
and disclosures. Existing dropdown return-focus behavior is covered by a UI
test; sheet-to-picker transitions retain their original return target, and
missing return targets fall back to the remembered chat owner. Help and workspace
hints share the navigation command definitions; session badges show bare numbers
in normal mode. Settings distinguish optional direct shortcuts.

Transcript cursor and `v` selection are deferred. Composer Escape retains its
existing steer/double-Escape-abort behavior; use double `Ctrl+g` to leave the composer.

## Interaction contract

Farcaster owns a chat normal mode, not a global Vim mode. Embedded Neovim and
terminal retain their own input semantics. Clicking an incidental control must
not silently change the primary keyboard owner.

| Chat normal keys | Action |
| --- | --- |
| `j` / `k` | Scroll transcript down / up by one reading-line height |
| `Ctrl+f` / `Ctrl+b` | Scroll down / up by the transcript viewport height |
| `Ctrl+d` / `Ctrl+u` | Scroll down / up by half the transcript viewport height |
| `i` / `a` | Focus composer and enter insert mode |
| `v` | Deferred: transcript visual selection |
| `/` | Open session search (not transcript search) |
| `Space e` | Show/focus embedded editor |
| `Space t` | Show/focus terminal |
| `Space j` / `Space k` | Next / previous session |
| `1`–`9` | Jump using existing visible-session numbering |
| `0` | Existing first-unsubmitted-draft action |
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

Escape does not leave the composer. Preserve the existing apply-steer /
double-Escape-abort behavior in `src/app/composer/submissions.rs`, along with
existing menu/dialog dismissal. Double `Ctrl+g` (or macOS `Cmd+g`) is the consistent
return-to-chat-normal command across composer and embedded surfaces. A single
chord activates app commands in place for one second: `Ctrl+g 2` selects session
2; `Ctrl+g Space e` opens the editor. Space renews the timeout; a command, Escape,
unknown continuation, or ownership/session/surface change clears activation.
New sessions explicitly enter composer insert mode.

In normal mode Escape cancels a pending leader without a destructive action.
If visual selection is implemented, Escape should clear it and stay in normal.

Render `NORMAL · TRANSCRIPT`, `INSERT · COMPOSER`, or `VISUAL · …` on the left
of the new status strip in `src/app/views/composer/footer/mod.rs`. Usage remains
on the right. Do not display invented mode state before ownership is wired up.
Embedded surfaces show their own mode indicators.

## 3. Space leader and session search

`src/app/ui/navigation/shortcuts.rs` owns normal-mode command/scroll definitions,
shared by routing and help. `src/app/ui/navigation.rs` owns the pending-leader
state and routes commands through existing backend-neutral app methods.
`src/app/ui/keybindings.rs` retains optional direct shortcuts. No mutually
exclusive keymap profile is required.

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

## 4. Transcript scrolling now; cursor and visual selection deferred

First inspect and extend the existing virtualized list/selection machinery in
`src/app/views/transcript/list.rs`, `src/app/views/transcript/list/`, and
`src/app/ui/keyboard.rs`. Reuse clipboard serialization and existing mouse
selection behavior; do not introduce a disconnected keyboard-only selection.

The current contract is scrolling only, not block or text cursor movement.
`j/k` scrolls one reading-line height without snapping to rendered text; paging
uses the actual transcript viewport, not the whole window. Composer, menus,
terminal, and Neovim retain their own keys. Modified paging cancels a pending
leader; unmodified leader `j/k` continues to switch sessions.

A future visual-selection contract still needs confirmation (blocks versus
rendered lines). Do not make useful scrolling depend on that larger project.

Use stable message/block identities rather than raw virtualized row indices.
Reveal the active cursor, pause tail-follow on intentional navigation, preserve
anchors during streaming/reflow, and recover deterministically when a block
collapses/disappears. `y` copies the selected range and returns to normal.
Provide an explicit return-to-live-tail command.

Tests: code/tool blocks, cross-message selection, virtualization gaps, streaming
appends, resize/reflow, collapse/expand, clipboard content, empty transcript,
per-session cursor restoration, and existing mouse-selection regressions.

## 5. Embedded boundary and rollout

Decision: reserve `Ctrl+g` everywhere and add `Cmd+g` on macOS. Both activate
one app command without changing the embedded owner; only a
double chord returns to the chat pane in normal mode. Activation and all its
continuations are intercepted before embedded/input action dispatch. Drafts, editor
state, and terminal processes remain intact. Pending agent requests are not
answered or cancelled by navigation. Ordinary Escape and Space inside either
embedded surface remain untouched. Native Ctrl+g / Cmd+g behavior is deliberately
unavailable through these reserved chords; configurable bindings/pass-through
can follow separately.

Keep transport/input implementation under the existing editor/terminal
boundaries (`src/app/workspace/editor.rs`, `terminal.rs` and their integrations).
No Pi-specific transport or session semantics belong in modal state.

Navigation is active alongside optional direct shortcuts. Add visual selection
only after its cursor contract is agreed and tested. `docs/usage.md`, shortcut
help, and settings describe the shipped navigation behavior.

Validation: narrow unit tests for transitions/keymap/list first, then integrated
focus routing tests and `cargo check --bin farcaster`; always `git diff --check`.
Manually exercise Linux desktop/input-method conflicts, embedded Neovim and
shell passthrough, pointer controls, keyboard-only dialogs, and narrow windows.
