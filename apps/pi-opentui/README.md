# pi-opentui

This is the first proof for a new Pi front end built with OpenTUI, Solid, Bun,
and Effect. It runs beside the current `pi` command.

The current screen starts a saved Pi SDK session, loads and binds the normal
Pi extension set, and provides:

- a multi-line prompt;
- streamed assistant and tool rows;
- `Escape` to stop a turn;
- select and text dialogs for extension tools and sandbox approvals;
- a slash menu for built-in, extension, prompt, and skill commands;
- local `/help`, `/session`, `/compact`, `/model`, `/thinking`, `/new`, and
  `/resume` commands;
- saved transcript restore when a session changes;
- current model and thinking level in the top status line;
- extension notices, status, retry, and compaction updates;
- scoped extension, session, task, and terminal cleanup.

It does not yet render images or rich custom extension views, import sessions
from other projects, or replace the current `pi` command.

Run the checks and app:

```sh
bun install --frozen-lockfile
bun run typecheck
bun test
bun run smoke
bun start
```

Press `Ctrl+C` or `Ctrl+Q` to quit. Press `Escape` to stop a running turn.
Press `Enter` to send and `Shift+Enter` for a new line.
Enter `/` by itself to open the command menu.
Use `/model text` to limit a long model list.

See [`../../PI_OPENTUI_PLAN.md`](../../PI_OPENTUI_PLAN.md) for the build stages,
limits, and checks.
