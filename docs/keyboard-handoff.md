# Keyboard work handoff

Resume from WIP commit `06c370a` (`feat(ui): WIP focus ownership and navigation help`).
Design and current bindings: [keyboard-plan.md](keyboard-plan.md).
User-facing behavior: [usage.md](usage.md#keyboard-navigation).

## User decisions — keep these

- `Ctrl+g` everywhere, plus `Cmd+g` on macOS: return to **chat normal**, even
  from Neovim/terminal. Preserve drafts and running processes.
- `i` and `a` both focus composer, retaining draft/caret.
- **Bare** `0–9` switches sessions; not Space-number. `0` is the first
  unsubmitted draft. `/` searches sessions.
- Space `e/t/j/k`: editor / terminal / next session / previous session.
- Bare `j/k` scrolls; Ctrl `f/b` pages; Ctrl `d/u` half-pages. These belong only
  to chat normal. No transcript cursor or visual selection yet.
- Composer Escape **keeps** existing steer/double-Escape-abort behavior.
  Do not turn it into exit-to-normal; Ctrl+g does that.
- Incidental pointer controls should activate without taking keyboard ownership.
  Menus/dialogs temporarily own input and restore the previous owner. Preserve
  deliberate Tab navigation and intentional focus changes such as opening an editor.

## What is implemented

Navigation and scrolling were already working; the user tried them and liked
how they felt. Session-reset focus stealing was fixed by remembering the last
chat owner instead of always focusing composer when async session data arrives.

The latest WIP addresses focus ownership and shortcut presentation:

- `src/app/ui/primitives/button.rs`: shared `preserve_pointer_focus` mouse-down
  handler. Applied to custom action rows, attachment controls, suggestions,
  and dialog choices. Existing shared Button behavior already avoided pointer
  focus; icon/disclosure controls now reuse the helper.
- `src/app/ui/navigation.rs`: remembered-chat-owner fallback; badge visibility
  follows actual normal focus; unhandled Tab/Shift-Tab traverses chat controls.
  Composer's bound Tab action and embedded keys must remain unaffected.
- `src/app/workspace/surfaces.rs`: restore remembered chat owner rather than
  defaulting to composer; keep explicit composer-entry actions unchanged.
- `src/app/navigation/picker.rs`: sheet-to-picker replacement retains the sheet's
  original return target instead of substituting composer focus.
- `src/app/ui/navigation/shortcuts.rs`: shared normal command/scroll definitions
  for routing, help, and workspace hints.
- `src/app/views/root/keybindings.rs`: normal commands first, optional **Direct**
  shortcuts separately; leader sequences render as separate keycaps.
- Session badges display bare numbers in normal mode, not modifier-number.
  Settings explain that the direct modifier does not affect modal bindings.

## Next steps / known unfinished audit

1. Review the remaining unconditional composer **fallbacks** (not intentional
   composer-entry actions):
   - `src/app/session/archive.rs`
   - `src/app/session/deletion.rs`
   - `src/app/project/repository/mod.rs`
   - `src/app/workspace/editor.rs`
   Consider `preferred_chat_focus()` only where appropriate. Preserve a valid
   captured return handle and intentional ownership changes.
2. Manually validate nested dialogs, sheet-to-picker replacement, and menu items
   that open another dialog/editor. Closing an old menu must not steal focus
   from the newly opened surface. Also test dismissal after the original
   session/control disappears; stale return handles are not comprehensively handled.
3. Validate actual Tab/Shift-Tab through controls and modal focus traps. The
   new control test checks `window.focus_next`, not complete app-level Tab routing.
   Verify composer Tab still submits/queues as before, and inline title editing
   still accepts focus/selection despite parent-row pointer protection.
4. Verify normal-mode badges on focus changes and window deactivate/reactivate;
   rapid session switching must remain normal after async data arrives.
5. Inspect keyboard help/settings at narrow widths and on macOS. Direct shortcuts
   remain available; do not silently remove them or introduce mutually exclusive
   keymap profiles. The composer status-strip leader hint is still a literal
   string and could be derived from the shared definitions later.
6. Exercise Linux/macOS embedded return chords and passthrough in the real app.
   No manual UI validation was completed for the latest WIP.

Do not implement visual selection or change composer Escape as part of this
cleanup. Keep backend-neutral navigation outside Pi-specific adapters.

## Validation already run

Before the WIP commit:

- `cargo check --bin farcaster` passed during implementation.
- `cargo test --bin farcaster app::ui::` — 29 passed.
- `cargo test --bin farcaster app::navigation::picker::tests` — 3 passed.
- `cargo test --bin farcaster app::workspace::surfaces::tests` — 2 passed.
- `cargo test --bin farcaster app::views::session_rail::tests` — 12 passed.
- `git diff --check` passed after formatting.

`src/app/ui/primitives/focus_tests.rs` is a real GPUI control harness: clicking
an action row preserves its owner; dropdown selection and Escape restore it;
controls remain reachable through explicit focus traversal. It does not exercise
the complete application or nested dialog/session races.

## Repository discipline

Other agents have been changing embedded Neovim, workspace, and session code.
Always inspect current status/diffs before editing; never reset or undo their
work. Stage whole task files only and exclude unrelated staged files from commits.
Use the active Cargo environment/shared target directory; no Nix commands or new
dependency/cache directories. Follow `AGENTS.md` and always run `git diff --check`.
