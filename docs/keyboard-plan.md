# Keyboard design

Current bindings and timing: [Usage](usage.md#keyboard-navigation).
Validation and remaining work: [Handoff](keyboard-handoff.md).

## Focus contract

Keyboard focus has four main targets: the **composer**, the **transcript**,
**Neovim**, and the **terminal**. There is no app-wide NORMAL or INSERT mode;
each target keeps its own input semantics. Overlays such as menus, sheets,
pickers, and confirmations temporarily own input while visible and restore the
underlying focus when dismissed.

- New sessions, session switches, and explicitly returning to chat from the
  editor or terminal focus the composer, preserving its draft and caret.
- Background work — agent completions, async session updates, indexing — never
  steals focus.
- Within chat, `Ctrl+K` focuses the transcript and `Ctrl+J` focuses the
  composer. They only switch focus between chat targets; they do not intercept
  keys owned by Neovim, the terminal, or overlays. When an agent request
  replaces the composer, it belongs to the composer target.
- Pointer input: clicking the composer's blank input area focuses the
  composer; clicking transcript text focuses the transcript and places the
  cursor at the clicked position. Action buttons never reposition the
  transcript cursor.

An incidental pointer click on a control activates that control without taking
keyboard ownership beyond the action itself; Tab, text editing, and explicit
surface commands deliberately move focus.

Composer Escape keeps its existing semantics: steer / double-Escape abort
behavior while a run is active. Session and surface changes cancel any pending
prefix sequence without moving focus by themselves.

## Ctrl+G prefix

`Ctrl+G` is the single reserved prefix, honored even inside Neovim and the
terminal, which otherwise own every key including Cmd combos. Activating the
prefix never moves focus; it opens a one-second pending window for one
continuation:

| Keys | Action |
| --- | --- |
| `Ctrl+G Ctrl+G` | Return to the chat composer |
| `Ctrl+G e` | Open the editor |
| `Ctrl+G t` | Open the terminal |
| `Ctrl+G 1`–`9` | Switch to the numbered session (composer focus) |
| `Ctrl+G 0` | First unsubmitted draft |
| `Ctrl+G /` | Search sessions |
| `Escape` | Cancel the pending prefix; consumed |

Opening the editor or terminal is an explicit surface move and focuses that
surface; session switches and `Ctrl+G Ctrl+G` land on the composer. Escape or
an invalid continuation is consumed, never replayed into the input, editor, or
shell. Expiry does nothing and subsequent keys type normally. Session, surface,
or focus changes cancel a pending prefix.

## Direct shortcuts

Direct shortcuts are app-owned-context chords: they fire only while a chat
target or a chat overlay owns input, never inside Neovim or the terminal.

| Platform | Keys | Action |
| --- | --- | --- |
| macOS | `Cmd+T` / `Cmd+W` / `Cmd+1`–`9` | New / close / numbered session |
| macOS | `Cmd+E` / `Cmd+J` / `Cmd+G` | Editor / terminal / chat composer |
| Linux | `Ctrl+T` / `Ctrl+W` / `Ctrl+1`–`9` | New / close / numbered session |
| Linux | `Ctrl+G` prefix | Editor, terminal, and chat composer return |

macOS has no blanket `Ctrl`↔`Cmd` aliasing: the table above is the complete
direct-shortcut set, and `Ctrl+J` / `Ctrl+K` keep their chat focus roles
(composer / transcript) on every platform. The Settings direct-shortcut
modifier changes only these chords, never the `Ctrl+G` prefix or
`Ctrl+J`/`Ctrl+K`.

## Implementation boundaries

| File | Responsibility |
| --- | --- |
| `src/app/ui/navigation.rs` | Pending-prefix timeout, continuation routing, focus observers |
| `src/app/ui/navigation/shortcuts.rs` | Shared prefix/direct definitions and hints |
| `src/app/ui/keybindings.rs` | Direct shortcuts and input-context bindings |
| `src/app/ui/focus.rs` | Guarded return-focus restoration and scoped Tab traversal |
| `src/app/workspace/surfaces.rs` | Overlay priority, active-surface fallback, deferred focus |
| `src/app/navigation/picker.rs` | Picker replacement and original return-target preservation |

Intercept the prefix and its continuations before embedded/input dispatch.
Unknown continuations are consumed, not replayed into a new input or shell.
Navigation preserves drafts, buffers, running processes, and pending agent
requests. Keep this policy backend-neutral; transport stays behind adapters.

Confirmations have independent focus handles and render above sheets. Dismissal
follows visual stacking order and must not steal focus from a newer surface.
A missing return target falls back to the surviving overlay or the chat
composer; native surfaces restore their own focus. Only repair missing
focus—do not force an owner on every render. Unhandled Tab bubbles to the
innermost modal; input-bound actions retain precedence.

## Transcript cursor and visual selection

`src/app/views/transcript/list/keyboard.rs` owns cursor/anchor coordinates and
motions inside the existing virtualized list. Coordinates are rendered row and
Unicode grapheme index, independent of wrapping and source Markdown offsets.
The transcript owns a Vim-style cursor while focused: `j/k` moves by displayed
line with a preferred horizontal position, word motions navigate words, `gg`
jumps to the transcript top, `G` jumps to its end and resumes live following
outside visual mode, `v` selects characters, and `V` selects displayed lines.
`y` copies and exits; native Copy preserves selection. `Escape` clears the
selection and cancels pending motion/operator sequences. Selection pauses tail
following. Focus loss and session reset clear visual state; replacing selected
rows cancels stale anchors.

A small backend-neutral `TextLayout::capture` seam in vendored GPUI reports the
exact shaped text and geometry during subtree prepaint. The transcript uses
rollback prepaint (`Window::transact`) to measure virtualized destinations
without installing offscreen hitboxes or focus targets. Normal rendering and
Markdown remain unchanged. The existing palette supplies selection and caret
colors; the status reads `VISUAL` or `VISUAL LINE` during a selection.

Geometry is retained only for visible rows and endpoints. Copy lays out missing
selected rows on demand, then releases their geometry. Pointer text selection
keeps its existing path and replaces keyboard selection; clicking transcript
text focuses the transcript and places the cursor. Cursor keys are routed only
when the transcript owns focus, never through composer/Neovim/terminal input.
