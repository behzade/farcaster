# Farcaster product and reliability audit

Status: audit complete; retained fixes and follow-up cleanup are committed in focused chunks.
This report separates observed failures,
source-backed risks, and proposals. It does not claim exhaustive live coverage of
every UI path.

## Product conclusion

Prioritize session continuity: users need to know that their prompt, chosen
configuration, draft, and session survive a restart or backend failure. The audit
found more support for this work than for adding new panels or agent features.
The strongest open issue is a mismatch between a local transport write and
native backend acceptance. A second priority is bounded, cancellable startup.

Several smaller proven fixes are included in the working tree. The remaining
delivery work needs an explicit product and protocol contract; automatic replay
of all old rows would duplicate prompts that already reached the backend.

## Scope and method

The audit covers session creation and resume, prompt delivery and recovery, backend parity, composer persistence, session-family actions, catalog scale, transcript access, and verification infrastructure. Independent reviewers trace different paths; findings must survive a second review and, where practical, a regression test or measurement.

Baseline commit: `b7b30c8c40da4e03ec3e213a6f890284ab9eb450`. The checkout contains concurrent changes, including the title fix from the preceding task and separate OpenCode work. Results describe the tested checkout, not a clean release build. No existing native session history or authentication configuration is changed for this audit.

## Evidence ledger

| Check | Observed result | Limit |
| --- | --- | --- |
| Actual runtime title regressions | 3 passed, including deferred resume and all six declared backend identities | Test transports; not a live desktop resume |
| Initial non-ignored application tests | 834 passed, 6 failed, 8 ignored; 2.90 seconds test execution | Debug test binary; four persistence failures exposed identity issues, two expectations were stale |
| End-of-audit application tests | 870 passed, 0 failed, 13 ignored; 2.37 seconds at four test threads | 883-test checkout; ignored cases include 3 known failures, 2 synthetic benchmarks, and 8 pre-existing opt-in tests |
| Higher-concurrency repeat | Same 870 passed, 0 failed, 13 ignored; 2.54 seconds at 12 test threads | Repeat run, not native multi-session load coverage |
| After refinement | 869 passed, 0 failed, 13 ignored; 2.89 seconds at four test threads | The idle assertion now shares the retry regression; all three known-failure proofs still fail when explicitly run |
| Early Stop regressions | 3 passed: deferred runtime prompt, Codex start response, Codex started notification | Pipe-backed and runtime fixtures, not a live desktop Stop |
| Shared build cache | Focused Cargo build succeeded using the normal shared cache | Sandboxed build had failed when Ghostty fell back to a cache requiring a network fetch |
| Outbox snapshot | 41 `sending`, 14 `failed`, 0 `queued`; completed rows are deleted | This is a backlog snapshot, not a denominator for submission failure rate |
| Native-history cross-check of 41 `sending` rows | 31 confirmed persisted, 1 nonempty unconfirmed, 1 empty, 8 not checked through this method | Exact matching cannot establish all backend acceptance semantics |
| Desktop inspection | Computer-use package fails before app access with `ReferenceError: process is not defined` | No live visual claims follow from source or transport tests |

## Follow-up refinement

Shared target parsing, legacy lookup, and locator-index resolution replace
duplicate paths. Ancestor lookup now returns an optional result instead of
building errors that callers discard. Archive, move, and delete gather their
live-work inputs through one app method. The clone test lives beside
`SessionSummary`, not in the catalog algorithm module.

Tests reuse the prompt recorder and one Codex cancellation fixture. The idle
assertion moved into the retry test; distinct symlink, missing-file, legacy
reader/writer, collision, and delivery-boundary cases remain. Whole-file commits
separate test expectations, runtime continuity, path persistence, family safety,
catalog copying, and these notes. Unrelated work remains untouched and uncommitted.

## Changes made

### Preserve queued prompts and their order

Before the fix, two persisted `queued` rows for one target reached the same
runtime actor in order, but the second overwrote the first `DeferredPrompt`.
The first row remained in SQLite without another delivery attempt in that run.
The regression sent only the second prompt. This was a synthetic crash-recovery
fixture, not an observed queued-row incident in the local data snapshot.

The runtime now retains a FIFO of recovered prompts. A normal prompt waits for
the previous turn to settle, not merely for a write response. An explicit
in-flight flag survives an early `LoadState(isStreaming=false)` before the
backend emits its start event. Rejection fails its own row and allows later
work to proceed. Normal replay also waits through compaction, retry, or pending
input. The independent review checked three queued prompts and repeated early
state updates, which exposed a queue-rotation bug in the first implementation.

Sources: [`prompts.rs`](../src/app/runtime/prompts.rs),
[`projection.rs`](../src/app/runtime/projection.rs), and
[`outbox_recovery_tests.rs`](../src/app/runtime/outbox_recovery_tests.rs).

### Close two early Stop gaps

Source review found two separate cancellation windows. The shared runtime could
retain a deferred prompt after Abort and send it when startup state arrived.
Codex could also accept Abort after writing `turn/start` but before receiving a
turn ID, return success, and never send `turn/interrupt`.

The scoped fixes cancel the one not-yet-dispatched deferred prompt, preserving
its durable record as failed, and retain Codex's cancellation request until the
turn ID arrives. They do not invent bulk cancellation rules for later recovered
prompts. They also do not make synchronous backend construction cancellable;
the startup deadline issue below remains separate.

Sources: [`runtime cancellation`](../src/app/runtime/prompts.rs),
[`Codex worker`](../src/modules/agents/adapter/codex/worker.rs), and
[`worker regressions`](../src/modules/agents/adapter/codex/worker_tests.rs).

### Preserve drafts through path aliases

On macOS, `/var/...` and `/private/var/...` can name the same session. A
target-derived composer lookup did not normalize the path, missed the indexed
session, and returned success without saving the draft. The initial suite
exposed this through draft promotion and family deletion tests.

Identity lookup and creation now use the same path normalization for explicit
paths and `session:` targets. Project persistence also uses a shared normalized
identity. Tests cover symlinks, nonexistent synthetic locators, moves, deletion,
and draft promotion. A full-suite rerun caught workgraph authentication readers
that still compared raw project paths; those readers now normalize without
weakening backend or duplicate-ID ownership checks. Legacy reader compatibility
is covered separately from fresh writes.

Sources: [`identity.rs`](../src/app/infrastructure/persistence/identity.rs),
[`persistence_tests.rs`](../src/app/infrastructure/persistence_tests.rs), and
[`workgraph.rs`](../src/app/mcp_server/workgraph.rs).

### Keep active session families safe

The existing supervisor already blocked most live move/delete operations, so
the first UI-only finding was not evidence that every running session could be
deleted. The missing state was `retrying`: the actual reducer can set it while
`running` is false. Both the UI and supervisor now account for that state. UI
preflight reuses the archive family check, including descendants and local
pending submissions.

Sources: [`activity.rs`](../src/app/session/activity.rs),
[`family_commands.rs`](../src/app/runtime/supervisor/family_commands.rs), and
[`family_commands_tests.rs`](../src/app/runtime/supervisor/family_commands_tests.rs).

### Remove avoidable catalog work

Session summaries now share immutable search text through `Arc<str>`. This
preserves all search data and matching behavior while avoiding a deep text copy
for each summary clone. Adjacent queued search commands now collapse to the
newest query; catalog-changing commands and shutdown remain ordered barriers.

The benchmark uses synthetic metadata at the observed scale, not native prompt
text. Timings below are individual local samples around the shared-text change,
before the final legacy-path fixes, not latency percentiles:

| Rust path, synthetic fixture | Debug before | Debug after | Release after |
| --- | ---: | ---: | ---: |
| Clone 1,535 rows, 77.9 MiB search text | 5.21 ms | 0.40 ms | 0.29 ms |
| Build 792 archived roots, 27.1 MiB search text | 2.72 ms | 0.33 ms | 0.14 ms |
| Load and decode whole SQLite catalog | 130.93 ms | 138.76 ms | 122.24 ms |
| No-match tree filter | 268.88 ms | 258.49 ms | 2.67 ms |

The optimized measurements do not support the initial hypothesis that tree
filtering or archived-list construction dominates this workload. Whole-catalog
loading remains material and runs on the catalog actor, not the UI thread.
The unchanged catalog comparison took 7.06 ms in the release synthetic run;
native frame profiling is still needed before calling it a visible frame stall.

The final checkout's fresh release sample, after all path fixes, measured
431.10 ms for importing the fixture, 172.87 ms for catalog decode, 0.31 ms for
cloning, 2.76 ms for the miss filter, and 3.76 ms for unchanged UI comparison.
The full benchmark took 0.98 seconds. Path compatibility adds filesystem lookups;
these single runs on a busy host do not isolate their exact cost. The audit
improves clone cost and skips obsolete queued searches, but does not claim an
overall catalog-load speedup.
The final archived-root fixture also passed, measuring 0.98 ms for 792 roots.

Sources: [`SessionSummary`](../src/modules/sessions/contract.rs),
[`command_queue.rs`](../src/app/runtime/command_queue.rs),
[`catalog benchmark`](../src/app/infrastructure/persistence/catalog_scale_tests.rs),
and [`rail benchmark`](../src/app/views/session_rail/groups_scale_tests.rs).

### Restore meaningful test expectations

Two initial failures were stale tests, not product defects. `simplify-commit`
is an intentional prompt fragment. Cmd/Ctrl+E and Cmd/Ctrl+T are documented
workspace shortcuts from commit `63a23e14`. The expectations now match that
contract and still verify that app shortcuts do not take ordinary embedded
terminal/editor input. No shortcut behavior changed.

### Stop title generation on resume

The title regression came from a shared runtime decision, not Codex's native
title generator. Missing session metadata counted as an empty unnamed session;
the runtime saved that decision in a deferred prompt before resume loaded its
state. Codex often exposes no session name in this path. The later dispatch
could therefore generate another title for an existing conversation.

The old predicate fails the resume regression. The fixed runtime requires an
explicit new-session launch plus loaded, empty, unnamed state at dispatch.
Resume and fork stay ineligible even when a backend omits metadata. The shared
policy covers all six declared identities; Pi and Codex are the two backends
that currently opt into Farcaster automatic generation.

History shows `2f920f9d` introduced the deferred `auto_title` flag and retained
the permissive missing-state predicate; `8e4fa41c` moved generation to prompt
dispatch. This is source history, not a full historical app bisect.

Sources: [`prompts.rs`](../src/app/runtime/prompts.rs),
[`process.rs`](../src/app/runtime/process.rs), and
[`title regressions`](../src/app/runtime/prompts_tests.rs).

## Open priorities and acceptance criteria

### P1: preserve the chosen execution settings through recovery

Startup restores per-harness model and effort defaults, but applies them to the
initial draft command, not recovered draft actors sent `DeliverQueued` directly.
An unstarted recovered draft can therefore launch with backend defaults. The
outbox also stores provider/model/effort/service-tier columns that `QueuedPrompt`
does not read or replay. Existing native session resume may retain its own
configuration, so the strongest confirmed behavioral case is an unstarted draft.

The first proposed patch restored those values, but review rejected it: dispatch
must await all required settings, handle rejection, and preserve the latest
choice when responses arrive out of order. A prompt submitted while a model
switch is pending must snapshot the desired model, not stale session metadata.
Cancelling a recovered prompt must not apply its remaining settings to later
work. The experimental replay patch was removed; this remains open.

Acceptance tests must cross the real boundaries: choose settings, save a first
prompt while startup is incomplete, recreate the supervisor from the database,
and verify the native launch/turn settings. Repeat for every backend, rapid
model changes, partial legacy metadata, rejected settings, and cancellation.
Treat provider and model as one pair; never combine halves from different
snapshots. Preserve native resumed settings when no complete saved override
exists.

Sources: [`supervisor.rs`](../src/app/runtime/supervisor.rs),
[`session_controls.rs`](../src/app/runtime/session_controls.rs), and
[`outbox persistence`](../src/app/infrastructure/persistence/prompts.rs).

### P1: distinguish local dispatch from backend acceptance

Two executable reproductions fail as intended: the worker bridge emits a
successful prompt response before a fake worker's later rejection, and the
runtime deletes the SQLite outbox row on that early response. The latter
observed zero rows where the test required one. These remain explicitly ignored
known-failure tests; a passing default suite does not resolve this issue.

| Backend path | What the present response establishes |
| --- | --- |
| Pi | A correlated native Pi RPC response |
| Codex | A JSON-RPC write; the real `turn/start` response can arrive later |
| Claude, Cursor, Antigravity ACP | A queued `session/prompt` request; its response arrives later |
| OpenCode2 | HTTP prompt admission, a stronger server-intake signal |

Introduce a stable delivery-attempt token and separate **local dispatch** from
**native acceptance/rejection**. Keep the prompt durable until there is native
evidence. Do not use the generic `WorkerEvent::Started` as an acknowledgment:
ACP can emit it on local request queueing. Also do not simply block the composer
until ACP's terminal prompt response; that may arrive only when the turn ends.
The UI must remain able to accept steering and follow-ups while retaining a
durable record of an unconfirmed send.

Acceptance tests should inject delayed rejection, disconnect before/after
acceptance, duplicate acceptance events, and an app restart at each boundary.
Each prompt must end in exactly one visible state: queued, accepted, explicitly
failed, or outcome unknown. No attempt should disappear or silently resend.

To measure this work, retain local lifecycle counts and timestamps by backend:
dispatch attempts, confirmed acceptance, rejection, unknown outcomes, and recovery
age. Record no prompt content. The current completed-row deletion makes a
submission success rate impossible to calculate from the outbox snapshot alone;
do not build a reliability dashboard with that invalid denominator.

Sources: [`main_session.rs`](../src/modules/agents/adapter/main_session.rs),
[`bridge regression`](../src/modules/agents/adapter/main_session_tests.rs), and
[`runtime consequence regression`](../src/app/runtime/outbox_recovery_tests.rs).

### P1: make uncertain delivery recoverable without duplicate work

The captured database held 41 old `sending` and 14 `failed` rows, with no queued
rows. At least 31 of the 41 were found in native history. This is evidence of
stale bookkeeping, not a 41-prompt loss incident or a failure rate. Startup
currently reads only `queued`; the schema's `unknown` state has no recovery
flow. Move/delete protection also checks queued rows but not unresolved old
sends, as a third failing audit reproduction demonstrates.

Reconcile at the owning runtime's startup, using native request IDs or durable
history evidence. Do not rewrite state on every `StateStore::open`: multiple
actors open the same database while other submissions are live. Unmatched
attempts need a recovery view with the original prompt and explicit retry/copy/
dismiss choices. Missing evidence is not evidence of rejection. Family deletion
must account for unresolved attempts or make their removal an explicit decision.

See the [aggregate-data appendix](product-audit-2026-09-09-data.md) for cohort
definitions, private hash-match limits, and reproducible read-only queries.

### P1: bound and cancel backend startup

Codex synchronously waits for setup responses such as `model/list` before its
runtime command loop can resume. `CodexConnection::wait_response` has an
unbounded blocking read. During the live probe, an unavailable inherited MCP
and model-manager timeout prevented clean conformance. While blocked in setup,
the actor cannot process a later Shutdown command.

Add one startup deadline and cancellation path, identify the operation that
timed out, retain the queued prompt, and reap only the owned child process.
Test a child that stays alive but never responds, including cancellation during
initialize, model listing, and resume. Existing ACP/OpenCode deadlines are
useful comparison points; this specific unbounded read is Codex-specific.

Sources: [`connection.rs`](../src/modules/agents/adapter/codex/connection.rs) and
[`worker.rs`](../src/modules/agents/adapter/codex/worker.rs).

### P2: split search payload from routine catalog summaries

The live catalog contained about 69.4 million search-text characters. One
release sample took 122.24 ms to load the 77.9 MiB ASCII fixture. This models
broad text volume, not the live row shape, UTF-8 distribution, or cache state.
Coalescing removes obsolete queued searches but does not remove that cost.
The next experiment should retain lightweight summaries and query searchable
data separately, preserving ancestor/descendant inclusion. Compare this with
an indexed search design using the same corpus and query semantics before
choosing an implementation. Blind truncation would change which sessions users
can find and is not part of this audit's fix.

Measure latest-query response time under rapid edits, bytes decoded per query,
and UI catalog-apply time. Use distributions over repeated release runs; the
single-sample timings above are not a performance SLO.

### P2: improve runtime compatibility and inspection

The adapter targets `opencode2 serve --stdio`, not arbitrary upstream
`opencode`. The installed upstream 1.18.11 failed that handshake in 0.39 seconds
before session creation. Preflight should name the required capability and
detected executable/version instead of only reporting closed stdout. This is
a compatibility/onboarding finding, not proof of an OpenCode2 regression.

Other measured or source-backed opportunities are narrower: 191 of 376 indexed
Codex titles exceed 120 characters and came from native title fields; a concise
display label can preserve the full stored title. Single-file Read rows should
keep their direct-open convenience while exposing an explicit details control.
Queued-message previews could open a full read-only view before adding editing
or cancellation semantics unsupported by some backends.

## Areas that held up, and remaining coverage limits

- Child worker final delivery, failure/capacity release, and input round-trips
  passed focused tests. No new defect was established in that path.
- Repository refresh already has watcher debounce and one in-flight/one-pending
  scan. Git/JJ routing and stale-result tests passed. No push or pull was run.
- Transcript streaming uses incremental tail projection and virtual rows. A
  full composer-history scan per snapshot remains an unmeasured optimization
  candidate; it is not reported as a demonstrated frame regression.
- Visited, unarchived session actors can remain resident without a capacity
  policy. Archived documents can be evicted. The live app's 48 total threads
  could not be attributed to actors, so there is no demonstrated runaway leak.
- Workgraph ownership tests passed after the path-reader correction; duplicate
  backend IDs still cannot share an authenticated task identity.
- Native desktop inspection was unavailable because the computer-use package
  failed before accessing the app. Source and transport tests are not visual QA.
- The host also hit system-wide `Too many open files` during final checks,
  including a Git status read. A later read showed 15,342 open files against a
  system limit of 20,480. This does not identify the process that hit the limit;
  no global limits or unrelated processes were changed.
- No Linux desktop run, Nix build, or release publication was performed.

## Live adapter probes

| Probe | Outcome | What it proves |
| --- | --- | --- |
| Codex | Reached app-server; inherited MCP/model-manager startup blocked full conformance | Startup failure/timeout handling needs work; no full pass claimed |
| OpenCode via installed upstream executable | Closed during unsupported `--stdio` handshake in 0.39 s | Upstream executable is not a compatible substitute for required OpenCode2 |
| Claude ACP | Setup completed in roughly 215 ms; native watcher emitted `EMFILE`; full run inconclusive | Adapter reached native Claude; no completed-turn or resume pass claimed |

Only owned probe processes were stopped. The owned Codex thread was deleted
and its metadata absence checked. OpenCode failed before session creation.
Claude's process exited; its full test result was not recovered and native
history may remain under the harness's retention policy. Pi, Cursor, and
Antigravity were covered by source/unit checks, not new live account runs.

## Reproduction commands

```sh
cargo test --locked --bin farcaster app::runtime::prompts::tests -- --nocapture
cargo test --locked --bin farcaster -- --test-threads=4 --quiet
cargo test --locked --release --bin farcaster catalog_scale_tests -- --ignored --nocapture
cargo test --locked --release --bin farcaster archived_rail_lists_792_large_roots -- --ignored --nocapture
git diff --check
```

The three known-failure checks below deliberately return a failing test result;
they are excluded from the default suite, not fixed or treated as passing:

```sh
cargo test --locked --bin farcaster prompt_response_does_not_precede_worker_rejection -- --ignored --nocapture
cargo test --locked --bin farcaster bridge_acknowledgement_does_not_complete_the_outbox_row -- --ignored --nocapture
cargo test --locked --bin farcaster sending_prompt_is_not_treated_as_safe_to_delete -- --ignored --nocapture
```

Commands used the existing Cargo target directory and normal shared dependency cache. No Nix build or cache relocation was used.
