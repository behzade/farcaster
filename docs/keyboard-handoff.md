# Keyboard handoff

Bindings: [Usage](usage.md#keyboard-navigation).
Architecture and invariants: [Keyboard design](keyboard-plan.md).

## Status

The focus contract — four main targets (composer, transcript, Neovim,
terminal), no app NORMAL/INSERT modes, the reserved `Ctrl+G` prefix, and
app-owned direct shortcuts — is specified in the two documents above.

The root `chat_navigation.focus` handle is only the restoration ancestor.
Transcript ownership is a separate handle; overlay input and arriving agent
requests must not overwrite that owner or steal from the editor/terminal.
Desktop acceptance checks below have not been verified yet.

## Acceptance checks pending verification

- Chat normal/insert modes, `i`/`a`, bare `1`–`9` session switching, the Space
  leader (`Space j`/`Space k`), `NORMAL`/`VISUAL` status labels, and
  normal-mode session badges are gone; transcript motions attach to transcript
  focus instead of a chat mode.
- `Ctrl+G Ctrl+G` returns to the chat composer, including from the composer
  during a run.
- macOS `Cmd+G` is a single direct shortcut to the chat composer, not a prefix
  alias; there is no blanket `Ctrl`↔`Cmd` aliasing of the direct-shortcut set,
  and `Ctrl+J`/`Ctrl+K` keep their chat focus roles on every platform.
- Direct shortcuts fire only in app-owned contexts; on Linux, editor, terminal,
  and chat-composer return go through the `Ctrl+G` prefix.
- Neovim and the terminal receive every key, including Cmd combos, except the
  reserved `Ctrl+G` prefix; Escape and invalid continuations are consumed.
- New sessions, session switches, and explicit chat return focus the composer;
  background work never steals focus.
- Clicking the composer's blank input area focuses it; clicking transcript text
  focuses the transcript and places the cursor; action buttons never reposition
  the transcript cursor.

## Validation pending

Verified against the contract-aligned implementation:

- `cargo check --offline --bin farcaster` passed.
- `cargo test --offline --bin farcaster app::ui::`: 41 passed.
- `cargo test --offline --bin farcaster app::views::transcript::`: 122 passed.
- `git diff --check` passes.
- Sandboxed runs needed escalation for normal cache access: the first check
  blocked at the Ghostty cache/fetch before reaching Farcaster.

Still unverified: real desktop/native input integration (no Linux/macOS window
interaction or screenshot review). The routing review is ongoing. Desktop
checks:

1. Composer, Neovim insert, running terminal: `Ctrl+G Ctrl+G`/`e`/`t`/`1`–`9`,
   Escape and invalid continuations, prefix timeout expiry, held keys, and
   confirmation that Neovim/terminal receive every non-prefix key including
   Cmd combos.
2. Direct-shortcut scoping: macOS `Cmd+T`/`Cmd+W`/`Cmd+1`–`9`/`Cmd+E`/
   `Cmd+J`/`Cmd+G` only in app-owned contexts; Linux `Ctrl+T`/`Ctrl+W`/
   `Ctrl+1`–`9`; no blanket Ctrl aliases; `Ctrl+J`/`Ctrl+K` intact everywhere.
3. Focus ownership: new session, session switch, and explicit chat return land
   on the composer; background work never steals focus; overlays restore the
   underlying focus on dismissal.
4. Pointer behavior: composer blank-area click, transcript text click with
   cursor placement, action buttons preserving the cursor.
5. Transcript caret/selection appearance, held motions, large-chat yank
   latency (full selection measures rows synchronously), resize/stream
   updates, pointer selection takeover, IME/desktop conflicts, native
   clipboard, and narrow help/settings appearance.

Formatting note: `cargo fmt --all` is blocked by the missing vendored
`crates/util/Cargo.toml`; format changed Rust files directly and run
`git diff --check`. Follow `AGENTS.md`; preserve other agents' work, stage only
whole task files, and leave unrelated staged files untouched.
