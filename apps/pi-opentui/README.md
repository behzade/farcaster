# pi-opentui

This app provides the canonical `pi` command. The upstream Pi package remains
an SDK dependency, not a second installed UI.

The front end uses OpenTUI core, Bun, TypeScript, and Effect.

The OpenTUI client uses explicit component updates rather than a React or
Solid renderer. Shared snapshots, commands, search rules, and display data
have no OpenTUI types, so a later web or desktop client can reuse them. Layout,
focus, terminal input, and renderable life spans remain client-owned.

The current screen starts a saved Pi SDK session, loads and binds the normal
Pi extension set, and provides:

- a multi-line prompt;
- steering with `Enter`, queued follow-ups, and queue restore while a turn runs;
- clipboard image paths and temp-file storage for large text pastes;
- streamed assistant and tool rows;
- `Escape` to stop a turn;
- select and text dialogs for extension tools and sandbox approvals;
- a slash menu for built-in, extension, prompt, and skill commands;
- local `/help`, `/session`, `/compact`, `/model`, `/login`, `/thinking`,
  `/new`, `/resume`, and `/reload` commands;
- saved transcript restore when a session changes;
- current model, thinking level, token use, context use, and cost in the top
  status lines;
- extension notices, status, retry, and compaction updates;
- scoped extension, session, task, and terminal cleanup.

It does not yet render images or rich custom extension views, or import
sessions from other projects.

Run the checks and app:

```sh
bun install --frozen-lockfile
bun run typecheck
bun test
bun run smoke
bun start
```

The app reads Pi's `~/.pi/agent/keybindings.json` and reloads it with `/reload`.
Its Pi AI patch also exposes native Responses output items. The compaction
extension uses that hook to send only a compaction trigger on a matching Codex
WebSocket chain, and the status totals include the saved compaction usage.
By default, `Ctrl+C` clears the input, a second `Ctrl+C` quits, `Ctrl+D` quits
when the input is empty, and `Escape` stops a running turn.
Press `Enter` to send and `Shift+Enter` for a new line.
While Pi works, `Enter` steers the turn, `Alt+Enter` queues a follow-up, and
`Alt+Up` returns queued prompts to the editor. Stopping a turn also restores
its queued prompts. Prompts entered during compaction wait until compaction
ends. These keys follow Pi's keybinding file.
Press `Ctrl+V` to paste an image from the system clipboard, with text fallback.
Text pastes over 10 lines or 1,000 characters use a scoped temporary file.
The app creates these files in a private OS temp directory and removes the
directory on a clean exit. Pasting sends Pi the local path; file contents only
leave the machine if a later agent or tool request reads and sends them.
Enter `/` by itself to open the command menu.
Type after `/` to see a command match, then press `Tab` or `Enter` to
complete it. Use `Up` and `Down` to move through matches.
The command and model menus filter as you type. `/model text` opens the model
menu with that search filled in.

See [`../../PI_OPENTUI_PLAN.md`](../../PI_OPENTUI_PLAN.md) for the build stages,
limits, and checks.
