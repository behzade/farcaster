# Keyboard handoff

Bindings: [Usage](usage.md#keyboard-navigation).
Architecture and invariants: [Keyboard design](keyboard-plan.md).

## Implemented

- One-second Ctrl+G activation; double chord returns to chat normal. New
  sessions enter insert. Direct Cmd shortcuts remain supported on macOS.
- Guarded focus restoration and surviving-overlay/native-surface fallback.
  Independent confirmation handles, correct stacking/dismissal order, and
  deferred focus checks prevent older overlays from stealing a newer owner.
- Sheet/picker replacement retains the original return target. Draft and
  persisted-session search selections preserve chat normal consistently.
- Scoped modal Tab traversal preserves input-owned actions. Badge visibility
  checks window activation. Shared leader hints and wrapping shortcut rows
  keep help/status consistent at narrow widths.

## Validation

- `cargo test --bin farcaster app::ui::`: 37 passed, including GPUI focus,
  modal/menu, composer Tab, title editing, and direct Cmd regressions.
- Narrow-help layout, workspace, picker, session-rail, archive, and window
  activation tests passed; `cargo check --bin farcaster` passed.
- The real headless Neovim buffer/tab preservation test passed with approval
  outside the sandbox; Unix-socket listen is denied inside it. This does not
  validate embedded key delivery.
- Headless GPUI window handles return `Unavailable`, allowing macOS input
  component tests without fabricated native handles.
- Changed Rust files are formatted directly: `cargo fmt --all` is blocked by
  the missing vendored `crates/util/Cargo.toml`. Run `git diff --check`.

## Remaining desktop checks

No real Linux/macOS window interaction or screenshot review was performed.

1. Composer, Neovim insert, running terminal: single/double Ctrl+G, timeout,
   Ctrl+G 2, Ctrl+G Space e/t/j/k, ordinary Escape/Space, and held keys.
2. New-session insert and rapid switching during async loads; badges and
   remembered focus through window deactivate/reactivate.
3. Archive/delete/JJ confirmations over sheets/native surfaces, nested menus,
   and dismissal after the original control disappears.
4. Narrow help/settings appearance, IME/desktop conflicts, native clipboard.

Visual selection remains deferred. Follow `AGENTS.md`; preserve other agents'
work, stage only whole task files, and leave unrelated staged files untouched.
