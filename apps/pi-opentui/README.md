# pi-opentui

This is the first proof for a new Pi front end built with OpenTUI, Solid, Bun,
and Effect. It runs beside the current `pi` command.

The current screen starts an in-memory Pi SDK session, loads and binds the
normal Pi extension set, and provides:

- a multi-line prompt;
- streamed assistant and tool rows;
- `Escape` to stop a turn;
- select and text dialogs for extension tools and sandbox approvals;
- extension notices, status, retry, and compaction updates;
- scoped extension, session, task, and terminal cleanup.

It does not resume saved sessions, render images or rich custom extension
views, or replace the current `pi` command.

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

See [`../../PI_OPENTUI_PLAN.md`](../../PI_OPENTUI_PLAN.md) for the build stages,
limits, and checks.
