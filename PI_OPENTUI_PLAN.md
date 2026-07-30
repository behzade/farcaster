# Pi OpenTUI plan

## Goal

Build a new Pi terminal front end with OpenTUI and Solid while keeping Pi's
agent SDK as the source of truth for sessions, tools, models, extensions, and
compaction.

The first app will live in `apps/pi-opentui`. It will not replace the current
`pi` command until it can run real work with the same safety checks.

## Main choices

- Use Bun, TypeScript, OpenTUI, and Solid for the terminal app.
- Use the installed `@earendil-works/pi-coding-agent` SDK. Do not copy its
  agent loop or session code.
- Use Effect for all app work that can fail, wait, run at once, or needs clean
  shutdown.
- Put the Pi session, event feed, app state, and host services in Effect
  Layers.
- Use `@effect/platform` and `@effect/platform-bun` for host access when the
  app needs files, paths, the terminal, or child tasks.
- Keep Solid signals local to short-lived view state such as focus, open
  panels, selection, and text input.
- Keep long-lived state in Effect services. Give Solid a small read and
  subscribe bridge.
- Do not add `effect-cli` yet. Pi already has argument parsing and this first
  cut has no command tree. Add it only when the new app owns enough commands
  to gain from it.
- Pin all package versions. OpenTUI and its Solid bridge change fast.

## Layer graph

```text
Bun host layer
    |
    +-- App config
    |
    +-- Pi session
    |     +-- Pi SDK session
    |     +-- extension load result
    |     +-- scoped event subscription
    |
    +-- App state
          +-- session events
          +-- commands: prompt, stop, quit
          +-- read and subscribe view
    |
    +-- OpenTUI renderer
          +-- scoped terminal setup and restore
          |
          +-- Solid root
                +-- app state read and subscribe view
                +-- local focus and input state
```

Each Layer must state what it owns and how it stops. A session scope removes
its listener, sends `session_shutdown` to extensions, and calls
`session.dispose()`. The root scope stops all fibers before OpenTUI gives the
terminal back.

## Source layout

```text
apps/pi-opentui/
  package.json
  bun.lock
  bunfig.toml
  tsconfig.json
  src/
    main.tsx
    app.tsx
    runtime.ts
    smoke.ts
    services/
      app-config.ts
      app-state.ts
      commands.ts
      extension-ui.ts
      pi-session.ts
      transcript.ts
      ui-renderer.ts
  test/
    app.test.tsx
    app-state.test.ts
    pi-session.test.ts
    transcript.test.ts
```

Keep files split by ownership, not by type. Do not add a wrapper when a direct
call has the right life span and error type.

## Work stages

Current progress:

- Stage 1 is complete in commit `8b3a2e89`.
- Stage 2 is complete with a prompt, streamed text and tool rows, stop support,
  extension status, retry and compaction notices, and Effect-backed approval
  dialogs.
- Stage 3 has started with command discovery, a slash menu, built-in command
  routing, safe rejection of unknown commands, saved sessions, and `/new` and
  `/resume` session replacement. Model and thinking-level choice also use the
  Pi SDK state, and the status line shows Pi's token, context, and cost data.

### 1. Prove the base

Build a small screen that:

- starts an in-memory Pi session for the current directory;
- uses the normal Pi agent directory, so it loads the current extension set;
- lists the active tool names and extension load faults;
- counts Pi session events through one scoped feed;
- quits cleanly and restores the terminal.

Exit checks:

- TypeScript passes.
- Effect tests prove session cleanup and event delivery.
- An OpenTUI render test proves the screen can mount.
- A local smoke run loads the same sandbox, compaction, and other extension
  packages without writing a session file.

### 2. Run one real turn

Add the prompt box and a small message list. Send prompts through the Pi
session service. Show text, tool starts, tool results, errors, and stop state.

Use an Effect queue for UI commands and one scoped fiber for the Pi event feed.
Do not call Pi with loose `async` handlers from Solid.

Exit checks:

- prompt, stop, retry, and quit work;
- streaming text updates without rebuilding all old rows;
- the sandbox approval path still blocks work until the user decides;
- server compaction events appear and do not stall the view.

### 3. Match core Pi use

Add session resume and switch, model and thought level choice, command search,
file mention completion, paste, image input, and the footer data needed for
normal work.

Create an OpenTUI form of Pi's extension UI context. Map each action on purpose.
Do not import the old `pi-tui` widgets.

Exit checks:

- every UI action used by the installed extensions either works or gives a
  clear "not yet supported" fault;
- current sessions open with the same Pi session manager;
- extension commands and shortcuts work from the new input.

### 4. Make long sessions fast

- Keep only visible transcript rows mounted.
- Store raw Pi messages outside Solid.
- Derive view rows once and key them by stable message and tool call IDs.
- Parse and color large code and diff blocks off the input path.
- Cancel work for rows that leave the view.
- Cache parsed code by content, language, theme, width, and view mode.
- Measure start time, key-to-draw time, scroll frame time, and memory on a long
  saved session.

Exit checks:

- input stays smooth during a large diff;
- scrolling a long session does not mount all old rows;
- idle work stops when its row or session scope ends;
- the cache lowers parse work and has a fixed size.

### 5. Ship beside current Pi

Add a Nix package and a separate `pi-next` command. Keep `pi` as the default.
Run both against the same extension bundle and saved sessions.

Change the default only after the new app passes a written feature list and
the safety tests. Removing the old UI is a later task.

## Extension plan

Pi extensions do three different jobs:

1. Core hooks and tools. Pi's SDK can load these now. This includes the
   sandbox tool path and most compaction work.
2. Commands and key bindings. The new input must route these through Pi.
3. UI calls and custom renderers. These use `pi-tui` types today and need an
   OpenTUI adapter.

The first stage proves group 1. Stage 2 binds an `ExtensionUIContext` adapter
for dialogs, notices, and status text. Stage 3 must add commands, key bindings,
widgets, and custom views.

The sandbox remains below the view layer. The new UI must not run tool calls
on its own or skip Pi's hook chain. The remote compaction extension also stays
in Pi's event and session path.

## Error and task rules

- Give each host or SDK fault a tagged error with the original cause.
- Use `Effect.tryPromise` only at SDK calls that return promises.
- Use Effect fibers, queues, streams, schedules, and scopes for wait and
  parallel work.
- Do not use bare `async` or `Promise.all` in app services.
- A Solid event handler may only update local view state or submit a command
  to the Effect app state service.
- Report extension load faults in the UI and logs.
- A failed background task must end in app state; do not leave a lost promise.

## Risks

- Pi's extension UI types import `pi-tui`. The core SDK does not remove this
  link. The first adapter covers plain dialogs, notices, and status text, but
  widgets and custom views still need OpenTUI forms.
- A package update can break OpenTUI's JSX bridge or native binary. Exact pins
  and a render test limit this risk.
- The Pi fork may not exist in the public package source used by Bun. The Nix
  package must give the app the exact SDK build that provides the current
  tools and patches.
- A retained renderer alone will not fix long chat speed. Row life span,
  parse work, and view size need their own limits and tests.
- Two front ends can drift in keys and commands. Keep command meaning in the
  Effect app state layer and keep the view thin.

## Review and rollback

Each stage ends with type checks, unit tests, a render test, and a smoke run.
Review cleanup, cancellation, event order, extension faults, terminal restore,
and any path that can skip sandbox checks.

The new command stays separate, so rollback means using `pi`. Do not change
saved session data or extension formats just for this UI.
