# Agent process metadata

New Pi RPC processes receive launch-time `FARCASTER_PROCESS_*` environment
variables on macOS and Linux. Commands inherit these unless a shell, sandbox,
or tool explicitly filters its environment. Existing running sessions need to
be restarted to receive the labels.

| Suffix | Value |
| --- | --- |
| `APP_PID` | PID of the launching Farcaster application |
| `BACKEND` | `pi` |
| `PROJECT` | Project directory |
| `ROLE` | `session`, `worker`, or `catalog` |
| `LAUNCH` | `new`, `resume`, `fork`, or `catalog` |
| `WORKER_ID` | Farcaster public caller/worker ID, including main sessions |
| `WORKER_NAME` | Farcaster caller/worker name at launch |
| `PARENT_WORKER_ID` | Parent Farcaster worker ID, when known |
| `PARENT_SESSION` | Parent Pi session locator, when known |
| `RESUME_FILE` | Session file supplied for resume only |
| `FORK_SOURCE` | Source file supplied for fork only; not the new session ID |

Optional fields are removed when absent so an inherited environment cannot
mislabel a new launch. Caller authorization tokens and task prompts are not
included. Names and paths are diagnostic information visible to users allowed
to inspect the process environment; do not put secrets in worker names.

These labels identify the originating launch, not current activity or live
session state. Pi itself supplies `PI_SESSION_ID`, `PI_SESSION_FILE`,
`PI_PROVIDER`, `PI_MODEL`, and `PI_REASONING_LEVEL` to its bash tool at command
launch. Those are the appropriate values for the current native session/model.

## Inspecting

On Linux, inspect a specific agent/action PID (subject to procfs permissions):

```sh
tr '\0' '\n' < /proc/PID/environ | grep '^FARCASTER_PROCESS_'
```

Linux process monitors with environment inspection can also display these
fields. On macOS, `ps eww -p PID` can expose the launch environment, subject to
OS permissions. **That command also prints unrelated environment variables,
possibly credentials: do not paste its unfiltered output into reports.**

From inside an agent action on either platform:

```sh
printenv | grep '^FARCASTER_PROCESS_'
```

Environment labels do not rename executable/process titles or add Activity
Monitor columns. macOS may continue grouping these processes under Farcaster
in Energy view. CPU view with individual PIDs separates the app's own CPU
usage from agents. No processes are detached, and lifecycle/cleanup semantics
are unchanged. Other backends are not yet annotated by this Pi adapter feature.
