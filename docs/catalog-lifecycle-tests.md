# Catalog lifecycle integration tests

These tests exercise the real supervisor, backend discovery, adapter subprocess
launch, protocol exchange, catalog translation, persistence, and runtime snapshot
publication. They do not call the catalog timeout helper or inject completed
catalogs into the runtime store.

## Run

Run all catalog lifecycle checks:

```sh
cargo test --bin farcaster catalog_lifecycle_tests --offline
```

The four original acceptance tests reproduced unsolicited startup, failed
reselection recovery for Claude and Antigravity, and ACP cross-backend blocking.
They now run without ignores alongside the working-behavior checks. Do not invert
their assertions or use `should_panic` to accommodate regressions.

## Isolation and control

Each test re-executes only itself in a separate process with a cleared environment,
empty application storage, and a PATH containing only the selected fixture
executables. It does not change the parent environment or use installed agents,
real credentials, browser login, or model usage. The restart test keeps its own
temporary application storage across two supervisor instances.

The std-only Rust fixture relays stdin/stdout through a local Unix socket. The
test supplies native-Claude or ACP responses; production adapters still launch
the process and encode, read, correlate, and translate messages. No protocol
fields enter production backend-neutral code.

Tests hold replies at a named request to control ordering. Three-second watchdogs
bound individual waits; a fifteen-second parent watchdog bounds each isolated
test. These are failure guards, not performance targets or a fake retry clock.
The fixture exits when its controller closes, including after a test crash.
Accepted sockets explicitly use blocking I/O on macOS.

## Implemented checks

| Check | Evidence |
| --- | --- |
| Antigravity happy path | The actual adapter publishes the fixture model in a runtime snapshot. |
| Claude happy path and restart | Load models through the adapter, restart, then read them from disk while the fresh response remains held; a failed refresh preserves them. |
| Native-Claude isolation | Claude publishes models while an Antigravity initialization response remains held. |
| Passive startup | Startup must not launch either unselected backend during the observation window. |
| Claude recovery | Publish the injected error, reselect in the same project, then require a new process and a successful model snapshot. |
| Antigravity recovery | The same recovery sequence through ACP. |
| ACP isolation | Hold one ACP initialization; the other ACP backend must finish without releasing it. |
| Picker request recovery and coalescing | After failure, send the same load command used by picker opening ten times; require Loading then Loaded, with only one new process. |

The ACP isolation test uses Cursor and Antigravity because both share the ACP
process cache. Native Claude no longer uses that cache. Separate happy-path and
isolation controls help distinguish a broken fixture from a product failure.

## Remaining coverage

This is the first regression set, not the whole reliability plan. Add these with
the lifecycle interfaces they need:

- GPUI picker actions for Retry, Sign in, and Cancel; tests here send runtime
  commands and stop at snapshots, not rendered controls.
- Explicit sign-in permission, late authentication completion, and cancellation.
  The current ACP fixture follows the existing unconditional authentication
  exchange. It must follow the new explicit action when that action exists.
- Retry scheduling with an injected clock, retry budgets, and typed
  authentication/permanent/transient failures.
- Stale-generation rejection, account/path invalidation, and selection retention.
- Stage deadlines, bounded worker counts, shutdown during each stage, child-tree
  reaping, stderr flooding, malformed frames, and unsupported cleanup.
- Active-chat prompt-count protection and model/effort/service-tier retention.
- Fish environment and executable discovery recovery. The current test build
  bypasses project login-shell capture, so these tests cannot prove that path.
- Resource soak checks and opt-in live/GUI checks against installed versions.

Before claiming the full reliability plan complete, also complete the UI,
authentication, cancellation, and resource checks above. A three-second no-launch
observation does not prove that no later scheduled task
can launch a backend; add a scheduler-idle barrier with the new coordinator.
