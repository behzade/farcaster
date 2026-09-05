# Usage and configuration

[← README](../README.md)

## Keyboard navigation

`Ctrl+G` returns from any surface to **chat normal mode**. macOS also accepts
`Cmd+G`. These are reserved by Farcaster, including in the terminal and Neovim.

In chat normal:

| Keys | Action |
| --- | --- |
| `i` / `a` | Focus composer, preserving draft and caret |
| `0` / `1`–`9` | First unsubmitted draft / numbered session |
| `/` | Search sessions |
| `Space e` / `Space t` | Editor / terminal |
| `Space j` / `Space k` | Next / previous session |
| `j` / `k` | Scroll transcript down / up |
| `Ctrl+f` / `Ctrl+b` | Page down / up |
| `Ctrl+d` / `Ctrl+u` | Half-page down / up |
| `Escape` | Cancel pending Space leader |

Session badges show bare numbers while chat normal owns input. Pointer clicks
on incidental controls do not take keyboard ownership. Menus/dialogs temporarily
own input and restore their return target when dismissed; an explicit action
such as opening the editor can intentionally move focus. Tab navigation remains
available for controls. Text fields and embedded tools keep their own keys.

Composer Escape retains apply-steer / double-Escape-abort behavior. Use `Ctrl+G`
to leave the composer instead. Visual selection is not implemented yet.

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
