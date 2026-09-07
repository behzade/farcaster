# Farcaster GUI state schema

Target design for the application SQLite database. The executable schema lives
in `src/app/infrastructure/persistence/schema.sql`; fresh databases and upgrades
use that same definition.

## Implementation status

The current change unifies session identity, project visibility, drafts, composer
state, prompt queue ownership, parent links, and prompt presentations. It also
keeps discovery from deleting sessions with saved state. Running status stays in
memory. Versioned legacy upgrades are separate from normal database opening.

The rest of this document describes the target architecture. Full conversation
recording and SQLite transcript replay are not implemented yet: history still
loads through adapters and startup still refreshes discovery. `session_ops` and
coverage fields are reserved for operation recovery and conversation recording.
Enqueue-time model columns exist, but dispatch does not use that snapshot yet.
User preferences and rail order still use SQLite UI settings, not TOML and
per-session ordering. This schema change alone does not complete those features.

Workgraph storage is a **separate** database (`workgraph/src/adapter.rs`) and is
out of scope.

**Decision:** proceed with this architecture. SQLite owns Farcaster’s
conversation record; adapters own backend resume state. Neither is a cache of
the other. The main work is lifecycle correctness, not more tables.

Defer until measured: FTS, event snapshots, extra metrics tables, extra
indexes.

## Product

Farcaster is a native desktop workspace for coding-agent sessions across
projects. Harnesses today: Pi, Codex, Cursor, OpenCode.

Users keep many sessions, grouped by project. A session is a conversation with
an agent: streaming assistant text, tool calls, permissions, and a composer.
Sessions can fork into parent/child workers. The session rail, search, and
transcript must stay fast when switching. Some harnesses (especially Cursor)
are slow or expensive to boot just to read history.

The app is **backend-neutral above adapters**. Pi jsonl, Codex threads, Cursor
ACP, OpenCode sessions are adapter artifacts. Core never interprets those files
and never uses a filesystem path as a session identity.

Two documents, different jobs:

| Document | Owner | Job |
| --- | --- | --- |
| Conversation record | Farcaster `session_events` | What this app showed: reopen, search, export, MCP |
| Resume payload | Harness adapter (`locator`) | What that backend needs for the next model call |

If they drift, the user still sees our record; the next prompt uses the
adapter’s resume state. Do not replay Farcaster events into a harness to
continue a turn.

Startup and transcript reopen must not require harness startup or filesystem
discovery.

## What lives where

**TOML (user prefs, copyable):** network proxy, shortcut modifier, built-in
MCP, worker-task routes. Ship a default file.

**SQLite (this schema):** projects, session identity, catalog fields, composer
drafts, prompt outbox, conversation events, window/rail UI state.

**Adapter:** `list` (import only), `load` (import/repair when coverage is
`unloaded`), `move` / `delete` / `rename`. Locators are opaque. Core never
`fs::` a session artifact.

**Live process / RPC:** running status, streaming. Not a DB column. A reopened
incomplete turn is **interrupted**, not running.

## Identity

- `sessions.id INTEGER PRIMARY KEY AUTOINCREMENT`. Deleted IDs are never
  reused.
- Bind the first locator with `UPDATE` on that row. Never `INSERT OR REPLACE`.
  Never insert a second session for the same conversation.
- `locator` NULL means **unbound** (no backend id yet), not “unsubmitted.” A
  submitted prompt can still be unbound until `sessionFile` (or equivalent).
- Adapter-mediated moves may change `locator`, never `sessions.id`.
- `UNIQUE (harness, locator)` applies to bound rows. SQLite allows multiple
  NULLs; that is intended for unbound drafts.
- `harness` is which adapter. Provider/model/effort/tier live on
  `session_models`.
- Projects: surrogate id, unique `path`, soft-delete `deleted_at`.

## Parentage

`sessions.parent_id` is authoritative. `ON DELETE SET NULL` so deleting a
parent keeps the child’s conversation record.

`backend_id` retains the adapter's session ID, which may differ from its locator.
`parent_backend_id` retains a discovered parent reference until its row arrives.
Indexing resolves these references within the same harness after saving the batch.

`worker_families` has **no parent column**. It exists only for metadata that
is not parentage (e.g. recovered child execution: provider/model/effort).
Child → parent is only `sessions.parent_id`. Do not pass one harness locator
to another backend.

## Conversation record

`session_events.body` is versioned JSON in Farcaster’s UI/event shape, not Pi
jsonl / ACP. Replay is **side-effect-free** (no process spawn, no adapter
writes, no outbox sends).

Coverage is explicit on `sessions.record_coverage`:

| Value | Meaning |
| --- | --- |
| `unloaded` | No Farcaster record yet (import stranger, or never attached) |
| `partial` | We have some events; gaps or crash possible |
| `complete` | Record is the full conversation this app knows |

Empty event rows or sequence gaps are **not** a coverage signal. Do not infer.

Correlate logical messages (stable ids in event bodies) so live echo + our
submission are not recorded twice.

Open: if coverage is `complete` or `partial`, replay SQLite. Spawn the harness
only when prompting or otherwise needing a live process. `adapter.load` only
for `unloaded` (and optional repair of `partial`).

Images/blobs stay off `session_events` (paths or a blob store). Attachments,
including unsent draft attachments, need durable references and an ownership
rule (session-scoped files deleted with the session).

Forks: **retain a defined fork boundary**. Do not copy inherited history into
the child event log. The child’s record starts at the fork; parent history
stays on the parent. (Adapter resume may still fork natively.)

Usage columns on `sessions` are **absolute totals** last observed (from live
RPC or import summary), not deltas.

## Prompt delivery

One atomic transaction:

1. Append the submission to `session_events`
2. Enqueue `outbox`
3. Clear the submitted composer draft

Correlate the backend echo with that submission (same logical message id).

Outbox states: `queued` → `sending` → accepted (delete the row after durable
local bookkeeping) or `failed`. **Unknown delivery** is distinct from
`failed`: reconcile or ask the user; never blindly resend.

Queued prompts use **enqueue-time** model settings (copy from `session_models`
onto the outbox row at enqueue). Send uses that snapshot, not whatever the
session model is at send time.

Prompt expansion happens in the adapter when building the provider request.
Persist what the user sent in `session_events`. No `prompt_presentations`
table.

## Adapter mutations

Move/delete (and rename if it touches artifacts) are recoverable:

1. Persist operation intent (`session_ops`)
2. Perform the adapter operation (idempotent or reconcilable)
3. Commit the result (new locator, deleted, failed)

Saving intent alone is not enough. Recovery replays or reconciles the adapter
call, then commits.

## SQL

```sql
PRAGMA foreign_keys = ON;

CREATE TABLE projects (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  added_ms INTEGER NOT NULL,
  deleted_at INTEGER,
  repository_backend TEXT
);

CREATE TABLE sessions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  harness TEXT NOT NULL,
  locator TEXT,
  backend_id TEXT,
  parent_backend_id TEXT,
  parent_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
  title TEXT NOT NULL DEFAULT '',
  first_user_message TEXT NOT NULL DEFAULT '',
  search_text TEXT NOT NULL DEFAULT '',
  timestamp TEXT,
  modified_ms INTEGER NOT NULL,
  archived_at INTEGER,
  rail_order INTEGER NOT NULL DEFAULT 0,
  record_coverage TEXT NOT NULL DEFAULT 'unloaded'
    CHECK (record_coverage IN ('unloaded', 'partial', 'complete')),
  message_count INTEGER NOT NULL DEFAULT 0,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  total_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL,
  UNIQUE (harness, locator)
);

CREATE TABLE session_models (
  session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  provider TEXT,
  model TEXT,
  effort TEXT,
  service_tier TEXT
);

CREATE TABLE session_events (
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  t INTEGER NOT NULL,
  schema_version INTEGER NOT NULL,
  body TEXT NOT NULL,
  PRIMARY KEY (session_id, seq)
);

CREATE TABLE worker_families (
  child_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  execution_json TEXT
);

CREATE TABLE outbox (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  submission_event_seq INTEGER,
  mode TEXT NOT NULL,
  message TEXT NOT NULL,
  display_message TEXT,
  invocation TEXT,
  images_json TEXT NOT NULL DEFAULT '[]',
  provider TEXT,
  model TEXT,
  effort TEXT,
  service_tier TEXT,
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK (state IN ('queued', 'sending', 'failed', 'unknown')),
  error TEXT,
  created_ms INTEGER NOT NULL
);

CREATE TABLE composer_sessions (
  session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  cursor INTEGER NOT NULL,
  selection_start INTEGER NOT NULL,
  selection_end INTEGER NOT NULL,
  history_json TEXT NOT NULL,
  updated_ms INTEGER NOT NULL
);

CREATE TABLE session_ops (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('move', 'delete', 'rename')),
  intent_json TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending', 'done', 'failed')),
  result_json TEXT,
  created_ms INTEGER NOT NULL
);

CREATE TABLE ui_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  window_placement_json TEXT
);

CREATE INDEX sessions_project_rail
  ON sessions(project_id, archived_at, rail_order);
CREATE INDEX sessions_parent ON sessions(parent_id);
CREATE INDEX outbox_session_state ON outbox(session_id, state, id);
```

## Tables

### `projects`

A working tree. `path` is the only filesystem path in this DB.

- `deleted_at` NULL = shown. Set on remove; clear to restore. Do not DELETE
  while sessions reference it.
- `repository_backend` is git vs jujutsu for that folder.

### `sessions`

Stable identity. One row per conversation.

- New composer → `INSERT` (`locator` NULL, `record_coverage` `complete` with
  empty events, or `unloaded` until first event — prefer `complete` empty).
- First backend id → `UPDATE locator`.
- Import → user-confirmed `adapter.list()`; coverage starts `unloaded` unless
  we also `load` into `session_events`.
- Startup → `SELECT` this table. No session-file walk.

`search_text` is a denormalized bag for Find session / rail filter.

### `session_models`

Last-used model for the session. Outbox copies these columns at enqueue.

- `effort` = reasoning/thinking.
- `service_tier` = request serving class (`fast` / `default` / …).

### `session_events`

Conversation record. `schema_version` versions `body`. Replay is pure.

### `worker_families`

Optional metadata for a child session. Parentage is `sessions.parent_id`.

### `outbox`

Durable prompt queue. `submission_event_seq` correlates to the event written
in the same transaction. Delete the row after confirmed acceptance.

### `composer_sessions`

Unsent input, caret, selection, up-arrow history.

### `session_ops`

Recoverable adapter mutations. Intent persisted before the adapter call;
`result_json` after (e.g. new locator).

### `ui_state`

Window placement. User prefs stay in TOML.

## Migration

Map every legacy key (`sessions.path`, `sessions.id`, `app_sessions.id`,
`drafts.id`, `composer_sessions.target`, `outbox.target` / `session_path`) to
**one** new `sessions.id`.

Preserve parent links, drafts, queued prompts, and original displayed user
text (`prompt_presentations`) before dropping legacy tables. Displayed user
text becomes events (and/or outbox metadata), not a join-by-string table.

Test crashes around: prompt acceptance, locator binding, backend move/delete.

## Explicitly dropped (legacy)

`meta` junk drawer, `drafts`, `app_sessions`, `sessions.path` as PK,
duplicate harness uuid, `is_running`, `file_size`, `harness DEFAULT 'pi'`,
string `target` / `session_path` keys, `excluded_projects`,
`prompt_presentations`, `worker_families.parent_id`.
