# Usage and configuration

[← README](../README.md)

## Keyboard navigation

`Ctrl+G` activates app keybindings for **1 second**, without changing focus or
leaving insert mode. macOS also accepts `Cmd+G`. These chords are reserved even
inside Neovim and the terminal. Double `Ctrl+G` returns to **chat normal mode**.

Activation is not the Space leader: `Ctrl+G 2` switches to session 2;
`Ctrl+G e` opens the editor, and `Ctrl+G t` opens the terminal.
Surface/session commands and transcript scrolling are available after activation.
Cursor selection requires chat normal ownership (double `Ctrl+G` first). A pending Space
or `g` refreshes the one-second timeout. A command ends activation; Escape or an unknown key
cancels it without forwarding that key to the input/editor/shell. Expiry does
nothing and subsequent keys type normally. Session/surface/focus changes cancel
pending activation. New sessions automatically focus the composer in insert mode.

In chat, `Ctrl+K` focuses the transcript in normal mode and `Ctrl+J` focuses the
composer. They only switch focus: pressing either in its target region does nothing.
They do not intercept keys in the editor, terminal, or app dialogs. When an agent
request replaces the composer, Ctrl+J focuses that request and Ctrl+K returns to
the transcript.

In chat normal:

| Keys | Action |
| --- | --- |
| `i` / `a` | Focus composer, preserving draft and caret |
| `1`–`9` | Numbered session (no motion counts) |
| `Ctrl+G 0` | First unsubmitted draft |
| `Ctrl+G /` | Search sessions |
| `Ctrl+G e` / `Ctrl+G t` | Editor / terminal |
| `Space j` / `Space k` | Next / previous session |
| `h` / `l` | Previous / next character (Unicode grapheme) |
| `j` / `k` | Cursor down / up one logical line |
| `gj` / `gk` | Down / up one soft-wrapped screen line |
| `w` / `b` / `e` / `ge` | Next word start / previous start / end / previous end |
| `W` / `B` / `E` / `gE` | Same motions for whitespace-delimited WORDs |
| `0` / `^` / `$` / `g_` | Line start / first nonblank / end / last nonblank |
| `g0` / `g^` / `g$` | Screen-line start / first nonblank / end |
| `+` / `Enter` / `-` | Next / previous line's first nonblank |
| `f<char>` / `F<char>` | Find character on the current line, forward / backward |
| `t<char>` / `T<char>` | Stop just before / after a character |
| `;` / `,` | Repeat / reverse the last character find |
| `{` / `}` / `(` / `)` | Previous / next paragraph or sentence |
| `%` | Matching bracket, including nested pairs |
| `H` / `M` / `L` | Top / middle / bottom visible line |
| `zt` / `zz` / `zb` | Align cursor line at viewport top / center / bottom |
| `/` / `?` | Literal rendered-text search forward / backward; Enter confirms, Escape cancels |
| `n` / `N` / `*` / `#` | Next / previous match; search current word forward / backward |
| `o` | Swap active and anchored ends of a visual selection |
| `gg` | Transcript top |
| `G` (Shift+g) | Transcript end; resume following unless selecting |
| `Ctrl+f` / `Ctrl+b` | Page down / up |
| `Ctrl+d` / `Ctrl+u` | Half-page down / up |
| `v` / `V` | Toggle character / logical-line visual selection |
| `y` | Copy selection; return to normal |
| `Cmd+c` (macOS) / `Ctrl+c` | Copy selection without leaving visual mode |
| `Escape` | Clear selection / cancel pending Space or g sequence |

All transcript motions also work in visual mode, preserving the selection anchor.
Arrow keys, Home/End, and PageUp/PageDown are supported. Motions operate on rendered
text, not Markdown source; this is navigation/selection, not a Vim editing engine.
Session badges show bare 1–9 while chat normal owns input. Pointer clicks
on incidental controls do not take keyboard ownership. Menus/dialogs temporarily
own input and restore their return target when dismissed; an explicit action
such as opening the editor can intentionally move focus. Tab navigation remains
available for controls. Text fields and embedded tools keep their own keys.

Composer Escape returns to chat normal when idle; while a run is active, it applies
queued steer / double-Escape aborts. Double `Ctrl+G` leaves the composer even during a run.
Empty sessions stay in the composer because there is no transcript to navigate.

The composer region (including agent requests replacing it) uses a muted blue-gray
border while focused, and a muted border when focus is elsewhere or the window
is inactive.

### Agent requests

Agent requests replace the composer without taking focus from transcript normal /
visual mode. Use `Ctrl+J` or `i` / `a` to focus a request, and `Ctrl+K` to return to
the transcript. In normal / visual mode, `y` remains yank and cannot grant permission.

Confirmation buttons show `[n] No` and `[y] Yes`; press the corresponding bare key
while the request has focus. Select requests show numbered choices. Enter, Space,
and held keys do not approve requests.

Pending requests do not block session switching: use direct session shortcuts or
`Ctrl+G` followed by a session number (or `Space j` / `Space k`). Switching away
leaves the request unanswered; returning restores it. Bare numbers in a request
choose options rather than switch sessions.

### Transcript selection

Chat normal shows its transcript caret while moving and for two seconds afterwards. Motions scroll it into view; in
`VISUAL` / `VISUAL LINE` they extend an inclusive selection from its anchor.
`V` selects logical text lines, including their soft-wrapped continuations. Copy uses rendered
text: prose, code, and visible labels/summaries, not Markdown delimiters, link
URLs, hidden disclosure contents, or image bytes. Character copying preserves
text across soft wraps; linewise copying ends with a newline.

Selection pauses live following, including `G` while selecting. After leaving
visual mode, `G` resumes following. Resize preserves grapheme anchors; replacing
selected content cancels selection rather than copying a stale range. Switching
sessions or moving keyboard focus elsewhere clears visual mode. Pointer text
selection remains available and replaces keyboard selection.

Keyboard help lists these commands first and **Direct** shortcuts separately.
Settings → **Direct shortcut modifier** changes only those direct shortcuts;
it does not change normal-mode keys, the Space leader, or `Ctrl+G` / `Cmd+G`.

## Access modes

Safety enforcement is delegated to the selected harness:

- Pi: Sandboxed or Full. Sandboxed preserves installed sandbox extensions such
  as `pi-nono`; Full sets `PI_NONO_DISABLED=1`.
- Codex: Sandboxed, Auto, or Full. Auto uses model-reviewed approvals.
- Cursor and OpenCode: Sandboxed or Full.

Unsupported modes are omitted from the selector.

Farcaster saves its own project trust decisions for repository commands. These
decisions do not change a harness's trust settings. Pi project-resource trust is
checked separately when opening a Pi session; other harnesses manage their own
trust. Existing Pi trust decisions do not grant Farcaster repository access.

## Embedded Neovim

Sessions in the same project share one Neovim process. Each session has its own
tabpage, preserving its open windows, splits, and cursor positions when switching
sessions. Buffers (including unsaved edits) and LSP clients are shared; editing
the same file in two sessions edits the same buffer.

Closing the editor surface returns to chat without terminating the shared
process. Use Neovim's `:qa` to quit the project editor. View state lasts for the
life of that Neovim process; it is not restored after restarting Farcaster.

## Prompt fragments

Files in [`prompts`](../prompts) are available in every harness. Type `$` to
complete a fragment. Fragments such as `$simplify $commit` expand in order
before submission.

## Built-in MCP

Built-in MCP is enabled by default for new sessions. It provides parent-child
workers, a project coordination notice board, and durable workgraphs. It can be
disabled under **Settings → Built-in MCP**.

When disabled, the MCP server does not bind a port. Switching it off stops the
listener and disconnects existing MCP clients; switching it on starts the server.

Up to eight child workers can be active at once. Idle children keep their sessions
for reuse without counting toward that limit. Messages to an idle child wait for
a free slot before starting another turn. Children send results explicitly with
`worker_send`; Farcaster reports child failures to the parent automatically.

### Worker task routing

**Settings → Worker tasks** lists your tasks beside their three judgment routes.
Use **Add task** to name a new definition; a task's **…** menu contains Rename
and Delete. Each route has dependent **Harness → Provider → Model → Effort**
selectors populated from the current project's cached harness catalogs.
Changing a harness or provider clears incompatible downstream choices. Changing
a model resets effort to the backend default.

Use **Reload choices** to reread catalogs discovered by the app. If a model is
not listed, the route's **… → Enter custom IDs** action opens an explicit editor.
Apply or cancel name/custom edits before saving Settings. **Save** persists all
task changes; **Cancel** discards them. Existing worker sessions are unchanged.

The initial task definitions are `read`, `implement`, and `review`, each with:

| Judgment | Responsibility | Initial route (Pi / openai-codex) |
| --- | --- | --- |
| `specified` | Parent supplies the procedure or exact checks | `gpt-5.6-luna`, high |
| `guided` | Child makes local decisions within constraints | `gpt-5.6-sol`, medium |
| `independent` | Child chooses an approach or challenges assumptions | `gpt-6-astra`, medium |

These are editable starter model IDs, not availability guarantees. Configure
routes for your installed harnesses and authenticated providers. Farcaster does
not inherit the parent's execution profile or silently fall back to it.
Task names classify work that is already being delegated; they are not agent
personas or permission restrictions. The schema contains no task-specific
recommendations about when to delegate.

Creating a child requires `task`; `judgment` defaults to `guided`:

```json
{"to":"check-parser","task":"review","judgment":"specified","message":"Check these three invariants…"}
```

Follow-up messages can omit both fields. A child's task, judgment, and resolved
route are bound on creation; conflicting classifications are rejected. Use a new
child name to select different routing. Deleting all definitions disables new
child creation but preserves messaging with existing children. Children cannot
select routing or spawn grandchildren. The tool schema exposes the saved task
names; clients that cache schemas may need to refresh their tools after edits.

Children may use a different harness from their parent. Farcaster persists those
family links separately from backend-native session ancestry. Harness-specific
trust, authentication, and access controls still apply; task routing does not
grant additional permissions or disable a harness's native delegation tools.

## Workgraph tools

Workgraph MCP tools use the authenticated caller's project and session. Agents
never supply a project, session ID, or session path.

- `workgraph_search({query?})`: list project tasks, optionally matching title or
  acceptance text. Results include task IDs, owners, blockers, dependencies,
  completion evidence, and `ready`, `blocked`, `claimed`, or `completed` status.
- `workgraph_patch({nodes: [{title, acceptance}], after?, before?})`: create an
  ordered chain or insert tasks beside existing task IDs. Creation does not claim.
- `workgraph_claim({task})`: atomically claim a ready task. Repeating your own
  claim succeeds; another owner's claim conflicts. Claiming links the task to
  your session's workgraph sidebar.
- `workgraph_release({task})`: relinquish your task so another session can claim it.
- `workgraph_complete({task, evidence})`: complete your task and report newly
  ready tasks. Successors are not automatically claimed.

Typical flow: search → claim → work → complete. To hand work off, release it;
then the other session claims the same task ID. A session can own one active
task at a time. Ownership and completion are durable across server restarts.
Existing recorded walk completions count as completed tasks in the shared graph;
old walk positions alone do not create exclusive claims.

## Configuration

- `FARCASTER_PI_PATH`: Pi executable
- `FARCASTER_CODEX_PATH`: Codex executable
- `FARCASTER_CURSOR_PATH`: Cursor Agent executable
- `FARCASTER_OPENCODE_PATH`: OpenCode executable
- `FARCASTER_PI_TITLE_MODEL`: Pi model for automatic session titles
- `FARCASTER_CODEX_TITLE_MODEL`: Codex model for automatic session titles
- `FARCASTER_DATA_DIR`: application database, project registry, and logs
- `FARCASTER_SHELL`: login shell
- `FARCASTER_GIT`, `FARCASTER_JJ`, `FARCASTER_NVIM`: tool executables

Application data defaults to `$XDG_DATA_HOME/farcaster`, or
`~/.local/share/farcaster` when `XDG_DATA_HOME` is unset. Run `make logs` to read
the application log.
