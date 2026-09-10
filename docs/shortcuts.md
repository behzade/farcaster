# Shortcuts

The main issue when designing the shortcuts for this app is that most Ctrl/Alt keybindings are required by the embedded terminal/Neovim to function properly. On macOS, this is not much of an issue because Cmd keybindings do not conflict with terminal input, but on Linux, most distros reserve the Super keys. So using Ctrl for app shortcuts inside the terminal was not an option on Linux.

To address this, the app has two sets of keybindings.

## Direct shortcuts

The first group works in the app's own views, outside the terminal/Neovim.

`mod`: Cmd on macOS, Ctrl on Linux by default.

Session and application shortcuts:

```text
mod+n -> new session
mod+number -> jump to session
mod+w -> dismiss dialog; close surface or draft; archive session
mod+shift+n -> add project
mod+shift+s -> set sandbox
mod+shift+m -> set provider/model/effort
mod+shift+h -> set harness (unsubmitted draft)
mod+shift+a -> restore session
mod+shift+p -> open action picker
```

When composer suggestions are visible, Ctrl+N selects the next suggestion instead.

Workspace shortcuts use the same letters as the leader commands:

```text
mod+e -> jump to editor
mod+t -> jump to terminal
Cmd+g -> jump to chat composer (macOS)
```

On Linux, use `Ctrl+G Ctrl+G` to return to the chat composer; `Ctrl+G` itself starts the leader. `F1`, `F2`, and `F3` are secondary shortcuts for chat, editor, and terminal in app views. `Ctrl+Tab` and `Ctrl+Shift+Tab` cycle workspace surfaces in app views.

## Leader shortcuts

The second group also works inside Neovim and the terminal:

`Ctrl+G` activates the leader for two seconds. Press the next key within that time; Esc cancels.

```text
<leader> <leader> -> jump to chat
<leader> e -> jump to editor
<leader> c -> comment on code in Neovim
<leader> t -> jump to terminal
<leader> number -> jump to session
<leader> j -> next session
<leader> k -> previous session
<leader> n -> new session
<leader> Shift+N -> start a task from code in Neovim
<leader> p -> add project
<leader> s -> set sandbox
<leader> m -> set provider/model/effort
<leader> h -> set harness (unsubmitted draft)
<leader> a -> restore session
<leader> Space -> open action picker
```

For session shortcuts, `1–9` select sessions and `0` opens the first unsubmitted draft.

Close dismisses an open dialog first, then closes the project work view, editor, or terminal. From chat with no dialog open, it discards the current draft or archives the current session. This applies to both `mod+W` and `Ctrl+G w`.

The action picker lists app commands, including workspace switching, previous/next session, close, abort, keyboard help, and quit. Numbered session jumps stay in the shortcut help; use Find session to choose a session by name. Sandbox choices and provider/model/effort choices happen inside the picker. Restore session lists archived sessions; selecting one restores it. Add project opens a folder chooser.

The runtime picker starts at the current model's effort choices, or at the current model if it has no effort choices. Use the Back button or Alt+Left to go back to models, then providers. Back restores the previous search, selection, and scroll position. Backspace also goes back when the search is empty.

The terminal/Neovim keeps its Ctrl keys. Super is unbound on Linux.

In Neovim, `Ctrl-G c` opens an app modal with the current file and cursor line
(normal mode), or selected buffer text and range (visual mode, including line
and block selections). Unsaved edits are included. Enter sends your comment and
code to the chosen chat; Shift+Enter adds a new line. **To** defaults to the current
chat and opens a searchable list of chats in this project, plus **New task**.
Sending keeps you in Neovim and leaves draft text and attachments intact. During
a run, comments steer when supported or queue as a follow-up. A failed send keeps
the comment in the destination's draft, after any existing text. Esc returns to the editor
without moving the cursor or changing the selection. File buffers only; captures
are limited to 2,000 lines and 128 KiB.

`Ctrl-G Shift-N` captures the same code for a new task. Add an instruction and
press Enter to send it in a separate chat, using the current harness and model.
Focus returns to Neovim; the original chat draft and attachments stay intact.
The confirmation offers **Open chat**. A failed start keeps the request in the
new chat's composer so you can retry. Both shortcuts use the same modal; you can
change the destination before sending.

## Composer and local keys

These keys require focus in the composer:

| Key | Behavior |
| --- | --- |
| Enter | Accept the selected suggestion; otherwise send a prompt, or steer during a run. Some command suggestions also submit on acceptance. |
| Shift+Enter | Insert a newline. |
| Tab | Select the next suggestion when suggestions are visible; otherwise send a prompt, or queue a follow-up during a run. |
| Shift+Tab | Select the previous suggestion when suggestions are visible; otherwise move focus backward. |
| Ctrl+N / Ctrl+P | Select the next/previous suggestion when suggestions are visible. |
| Up / Down | Select the previous/next suggestion; otherwise move through text. Up on the first line browses previous prompts; Down on the last line moves forward while browsing history. |
| Esc | Apply queued steering during a run, or press twice to abort when no steering is queued. |

In dialogs, Tab and Shift+Tab move focus and Esc dismisses the dialog. In the action picker, Tab/Shift+Tab and Ctrl+N/P select items; Enter chooses one. In project work, navigation keys such as `j`, `k`, and `/` require focus outside the search input; Esc also works within search. The in-app shortcut help groups keys by where they work.

## Transcript scratch buffer

Press `Ctrl-G v`, choose **Open transcript in Neovim** in the action picker,
or use the transcript context menu to open a snapshot in a Markdown scratch
buffer with message headings and the chat's activity summaries, without raw
tool input/output. Use Neovim search, movement, and yank commands; `"+y` copies
a selection to the system clipboard when your Neovim clipboard provider is available.
The buffer does not write to the conversation or a project file. Open the action
again for a fresh snapshot. `Ctrl-G Ctrl-G` returns to the composer.
