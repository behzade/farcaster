# pi-opentui

This is the first proof for a new Pi front end built with OpenTUI, Solid, Bun,
and Effect. It runs beside the current `pi` command.

The current screen starts an in-memory Pi SDK session, loads the normal Pi
agent directory, shows active tools and loaded extensions, counts session
events, and cleans up on exit. It does not have a prompt or chat view yet.

Run the checks and app:

```sh
bun install --frozen-lockfile
bun run typecheck
bun test
bun run smoke
bun start
```

Press `q` or `Ctrl+C` to quit. Press `Escape` to stop a running turn once the
prompt lands in the next stage.

See [`../../PI_OPENTUI_PLAN.md`](../../PI_OPENTUI_PLAN.md) for the build stages,
limits, and checks.
