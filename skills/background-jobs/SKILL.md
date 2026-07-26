---
name: background-jobs
description: Start, inspect, and interact with long-running shell commands without blocking the Pi agent. Use for dev servers, watchers, builds, tests, and commands that need later input.
compatibility: Requires tmux and an interactive Pi session with the bash tool.
---

# Background jobs

Use the helper through Pi's normal `bash` tool. This keeps command approval and
the OS sandbox in force.

Do not use this skill for a short command. Do not run `sudo`, a password prompt,
or a destructive command in the background.

Use a unique name that starts with `pi-` and contains only letters, digits,
periods, underscores, or hyphens. Keep the name for later calls.

Set this helper path in each call:

```sh
~/.pi/agent/skills/background-jobs/scripts/job.sh
```

## Start

Pass the command as one quoted argument. The call returns at once.

```sh
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh \
  start pi-app-dev /absolute/project/path 'npm run dev'
```

Continue other useful work after `start`. Do not poll in a tight loop.

## Inspect

```sh
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh list
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh status pi-app-dev
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh read pi-app-dev 200
```

`read` returns the last requested number of terminal lines. A finished job stays
available for reads until it is stopped.

## Send input

Send text without Enter:

```sh
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh \
  write pi-app-dev 'input text'
```

Send one line with Enter:

```sh
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh \
  line pi-app-dev 'yes'
```

Send terminal keys:

```sh
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh keys pi-app-dev C-c
```

## Stop

Stop only a job created for the current task:

```sh
bash ~/.pi/agent/skills/background-jobs/scripts/job.sh stop pi-app-dev
```

Report any job left running when the task ends.
