# Usage and configuration

[← README](../README.md)

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
