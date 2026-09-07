# Shortcuts

The main issue when designing the shortcuts for this app is that most Ctrl/Alt keybindings are required by the embedded terminal/Neovim to function properly. On macOS, this is not much of an issue because Cmd keybindings do not conflict with terminal input, but on Linux, most distros reserve the Super keys. So using Ctrl for app shortcuts inside the terminal was not an option on Linux.

To address this, the app has two sets of keybindings.

## Direct shortcuts

The first group works in the app's own views, outside the terminal/Neovim.

`mod`: Cmd on macOS, Ctrl on Linux by default.

These mimic common browser keybindings:

```text
mod+number -> jump to session
mod+w -> close surface or draft; archive session
mod+shift+n -> add project
mod+shift+s -> set sandbox
mod+shift+m -> set provider/model/effort
mod+shift+a -> restore session
mod+shift+p -> open action picker
```

## Leader shortcuts

The second group also works inside Neovim and the terminal:

`Ctrl+G` activates the leader for two seconds. Press the next key within that time; Esc cancels.

```text
<leader> <leader> -> jump to chat
<leader> e -> jump to editor
<leader> t -> jump to terminal
<leader> number -> jump to session
<leader> j -> next session
<leader> k -> previous session
<leader> n -> new session
<leader> p -> add project
<leader> s -> set sandbox
<leader> m -> set provider/model/effort
<leader> a -> restore session
<leader> Space -> open action picker
```

For session shortcuts, `1–9` select sessions and `0` opens the first unsubmitted draft.

The action picker also lists these actions. Sandbox choices and provider/model/effort choices happen inside the picker. Restore session lists archived sessions; selecting one restores it. Add project opens a folder chooser.

The runtime picker starts at the current model's effort choices, or at the current model if it has no effort choices. Use the Back button or Alt+Left to go back to models, then providers. Back restores the previous search, selection, and scroll position. Backspace also goes back when the search is empty.

The terminal/Neovim keeps its Ctrl keys. Super is unbound on Linux.
