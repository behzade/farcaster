# Keyboard design

Current bindings and timing: [Usage](usage.md#keyboard-navigation).
Validation and remaining work: [Handoff](keyboard-handoff.md).

## Ownership

Farcaster owns chat normal mode, not a global Vim mode. Composer, Neovim,
terminal, and temporary UI retain their own input semantics. An incidental
pointer click activates a control without taking keyboard ownership; Tab,
text editing, and explicit surface commands may deliberately move focus.

`Ctrl+G` activates one app command without moving focus. It is distinct from
the Space leader. Double `Ctrl+G` returns to chat normal; `i`/`a` enters insert.
`e`/`t` opens editor/terminal directly in normal mode. `gg` jumps to transcript
top; `G` jumps to its end and resumes live following. Space is reserved for
session navigation (`j`/`k`). New sessions enter insert automatically. Async
session updates preserve the remembered chat owner. Composer Escape keeps steer/double-Escape-abort behavior.

Direct shortcuts, including default macOS Cmd bindings, remain supported
alongside modal navigation. No mutually exclusive keymap profiles.

## Implementation boundaries

| File | Responsibility |
| --- | --- |
| `src/app/ui/navigation.rs` | Activation timeout, chat mode, command routing, focus observers |
| `src/app/ui/navigation/shortcuts.rs` | Shared normal/leader/scroll definitions and hints |
| `src/app/ui/keybindings.rs` | Direct shortcuts and input-context bindings |
| `src/app/ui/focus.rs` | Guarded return-focus restoration and scoped Tab traversal |
| `src/app/workspace/surfaces.rs` | Overlay priority, active-surface fallback, deferred focus |
| `src/app/navigation/picker.rs` | Picker replacement and original return-target preservation |

Intercept activation and its continuations before embedded/input dispatch.
Unknown continuations are consumed, not replayed into a new input or shell.
Navigation preserves drafts, buffers, running processes, and pending agent
requests. Keep this policy backend-neutral; transport stays behind adapters.

Confirmations have independent focus handles and render above sheets. Dismissal
follows visual stacking order and must not steal focus from a newer surface.
A missing return target falls back to the surviving overlay or remembered chat
owner; native surfaces restore their own focus. Only repair missing focus—do
not force an owner on every render. Unhandled Tab bubbles to the innermost
modal; input-bound actions retain precedence.

## Deferred

Transcript scrolling is implemented; keyboard cursor and visual selection are
not. Before adding them, agree on block-versus-line selection, stable anchors,
copy behavior, and return-to-live-tail. Reuse the existing transcript list and
selection machinery rather than introducing a separate keyboard model.
