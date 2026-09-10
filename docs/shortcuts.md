# Shortcuts

The main issue when designing the shortcuts for this app is that most Ctrl/Alt keybindings are required by the embedded terminal/Neovim to function properly. On macOS, this is not much of an issue because Cmd keybindings do not conflict with terminal input, but on Linux, most distros reserve the Super keys. So using Ctrl for app shortcuts inside the terminal was not an option on Linux.

To address this, the app has two sets of keybindings.

## Direct shortcuts

The first group works in the app's own views, outside the terminal/Neovim.

`mod`: Cmd on macOS, Ctrl or Super on Linux (unless reserved by the desktop). Explicit Ctrl shortcuts work on both platforms.

Session and application shortcuts:

```text
mod+q -> quit (confirms if work is active)
mod+n -> new session
mod+number -> jump to session
mod+w -> close surface or session
mod+shift+n -> add project
mod+shift+s -> set sandbox
mod+shift+m -> set provider/model/effort
mod+shift+h -> set harness (unsubmitted draft)
mod+shift+a -> restore session
mod+shift+p -> open action picker
```

Workspace shortcuts use the same letters as the leader commands:

```text
mod+e -> jump to editor
mod+t -> jump to terminal
Cmd+g -> jump to chat composer (macOS)
```

`F1` / `F2` / `F3` jump to chat / editor / terminal in app views. `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle them.

## Leader shortcuts

The second group also works inside Neovim and the terminal:

`Ctrl+G` activates the leader for two seconds. Press the next key within that time; Esc cancels.

```text
<leader> <leader> -> jump to chat
<leader> e -> jump to editor
<leader> c -> comment on code in Neovim
<leader> t -> jump to terminal
<leader> v -> open transcript in Neovim
<leader> w -> close surface or session
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
<leader> q -> quit (confirms if work is active)
<leader> Space -> open action picker
```

For session shortcuts, `1–9` select sessions and `0` opens the first unsubmitted draft. Ctrl+0–9 works in app views on both platforms.

Close dismisses a dialog or surface first; from chat, it discards the draft or archives the session.

Cmd+0–9 on macOS and Super+0–9 on Linux also work inside the terminal/Neovim. The Ctrl+G leader works on both.

## Composer and local keys

These keys require focus in the composer:

| Key | Behavior |
| --- | --- |
| Enter | Accept suggestion or send prompt; steer during a run. |
| Shift+Enter | Insert a newline. |
| Tab | Next suggestion or send prompt; queue during a run. |
| Shift+Tab | Previous suggestion or move focus backward. |
| Ctrl+N / Ctrl+P | Next/previous suggestion when visible. |
| Up / Down | Previous/next suggestion or move through text. Up on the first line opens prompt history; Down on the last line moves forward through it. |
| Esc | Apply queued steering; otherwise press twice to abort. |

In dialogs, Tab / Shift+Tab move focus and Esc closes. In the action picker, Tab / Shift+Tab or Ctrl+N / Ctrl+P select items; Enter chooses one.
