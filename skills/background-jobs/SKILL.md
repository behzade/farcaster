---
name: background-jobs
description: Start, inspect, and interact with long-running commands through Pi's background_job tool. Use for dev servers, watchers, builds, tests, and commands that need later input.
---

# Background jobs

Use the `background_job` tool. Do not call tmux or the bundled helper through
bash.

Do not use this skill for a short command. Do not run `sudo`, a password prompt,
or a destructive command in the background.

Use a unique name that starts with `pi-` and contains only letters, digits,
periods, underscores, or hyphens. Keep the name for later calls.

## Start

```json
{"action":"start","name":"pi-app-dev","command":"npm run dev"}
```

Set `cwd` only when the job must start in a subfolder of the current workspace.
The command runs in a fresh Codex sandbox. Request any needed network host
before starting the job. Continue other useful work after `start`; do not poll
in a tight loop.

## Inspect

```json
{"action":"list"}
{"action":"status","name":"pi-app-dev"}
{"action":"read","name":"pi-app-dev","lines":200}
```

`read` returns the last requested number of terminal lines. A finished job stays
available for reads until it is stopped.

## Send input

```json
{"action":"write","name":"pi-app-dev","text":"input text"}
{"action":"line","name":"pi-app-dev","text":"yes"}
{"action":"keys","name":"pi-app-dev","keys":["C-c"]}
```

`write` sends text without Enter. `line` adds Enter.

## Stop

Stop only a job created for the current task:

```json
{"action":"stop","name":"pi-app-dev"}
```

Report any job left running when the task ends.
