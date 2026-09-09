# Aggregate session-data evidence appendix

Captured during the 2026-09-09 audit while Farcaster was
running. All database access used SQLite read-only mode. Native transcripts
were read only to compare SHA-256 values in memory; this appendix contains no
prompt text, transcript text, hashes, project paths, session IDs, or error
strings.

## Snapshot scope and drift

The Farcaster database is
`/Users/behzad/.local/share/farcaster/state.sqlite3`. The first catalog
snapshot returned 1,534 sessions: 638 active and 896 archived. A later
measurement returned 1,535: 639 active and 896 archived. Here, "active" means
unarchived, not currently executing. Farcaster indexed
one new unarchived session while the audit ran. Counts below name their cohort and
are not a submission-failure rate.

The native stores examined were the configured Pi directory
`/Users/behzad/.pi/agent/sessions`, Codex rollouts under
`/Users/behzad/.codex/sessions`, and Codex metadata at
`/Users/behzad/.codex/state_5.sqlite`.

## Durable prompt states

At the queue snapshot, `outbox` contained 41 `sending` rows across 41
sessions, 14 `failed` rows across 10 sessions, and no `queued` rows. The
schema has `queued`, `sending`, `failed`, and `unknown`; successful delivery
deletes its row. Therefore there is no persisted `complete` or `dispatching`
cohort to count.

For the 41 stale `sending` rows, byte-exact SHA-256 comparison found 26 Pi
messages in the associated native user history and 5 Codex messages as a
scalar in the associated native rollout. One Pi row had an empty message, one
nonempty Codex row had no scalar match at the snapshot, and 3 Cursor plus 5
OpenCode rows were not checked. This proves stale outbox state, not 41 lost
prompts. It also means an automatic resend could duplicate at least the 31
confirmed-persisted rows.

Pi matching parsed JSONL `message` entries with `message.role == "user"` and
hashed the same visible-text projection used by the adapter. Codex matching
decoded the ID from Farcaster's opaque locator, selected its one native
rollout, and looked for the exact message hash among decoded scalar values.
The latter proves native persistence but is not an acceptance acknowledgement.
Cursor and OpenCode need their live protocol or a backend-owned read-only
history query, which this audit deliberately did not start.

The relevant code establishes the ambiguity:
[`begin_prompt`](../src/app/infrastructure/persistence/prompts.rs) writes `sending` before
[`dispatch_prompt`](../src/app/runtime/prompts.rs) calls the transport, while
startup asks only for [`queued_prompts`](../src/app/runtime/supervisor.rs).
[`complete_prompt`](../src/app/infrastructure/persistence/prompts.rs) deletes
the durable row. In the captured database, every persisted session event was a
`prompt_presentation`; there was no generic acceptance receipt to reconcile a
stale send.

Reproduce the aggregate state without selecting message or error columns:

```sh
FARCASTER_DB=/Users/behzad/.local/share/farcaster/state.sqlite3
sqlite3 -readonly "$FARCASTER_DB" <<'SQL'
SELECT state, count(*) AS rows, count(DISTINCT session_id) AS sessions,
       min(datetime(created_ms / 1000, 'unixepoch')) AS oldest_utc,
       max(datetime(created_ms / 1000, 'unixepoch')) AS newest_utc
  FROM outbox
 GROUP BY state
 ORDER BY state;

WITH queued AS (
  SELECT session_id FROM outbox WHERE state = 'queued'
)
SELECT count(*) AS queued_rows,
       count(DISTINCT session_id) AS queued_targets,
       count(*) - count(DISTINCT session_id) AS duplicate_target_rows
  FROM queued;
SQL
```

The live snapshot had zero queued rows and therefore no data evidence for the
separate single-`DeferredPrompt` overwrite failure. That failure was reproduced
with a synthetic persisted queue and fixed during this audit; it was not
observed in this live-data snapshot.

## Catalog size, text payload, and navigation

`dbstat` reported an 80.5 MB `sessions` table inside a 171.4 MB state database.
For the 1,535-row later snapshot, `search_text` held 69.36 million characters,
`first_user_message` 4.95 million, and `title` 4.34 million. The full stored
catalog query, with output discarded and a warm OS cache, took 160--170 ms in
five fresh SQLite CLI processes. This includes CLI startup and output-formatting
overhead and is not a lower bound on Rust execution. The main audit report gives
separate measurements of the actual Rust code with a synthetic comparable workload.

The archived rail was the largest root-list copy cohort: its 792 root sessions hold 27.73
million search-text characters, of which Pi roots hold 26.13 million. The
active rail has 6 roots and 671 search-text characters. This matters because
[`cached_sessions`](../src/app/infrastructure/persistence/sessions.rs) reads
the whole catalog, [`filter_session_tree`](../src/modules/sessions/core/catalog.rs)
does substring search in memory, and the picker clones a session summary
([`picker.rs`](../src/app/navigation/picker.rs)). The audit's shared-text
change removes repeated deep copies in session-summary clones. Measured release
root-list construction is small; the remaining substantial cost is catalog loading.

Pi bounds search text to 64 KiB, but that is still large at this scale. Codex
and OpenCode construct their search string without a shared bound; Cursor and
ACP omit the first user preview but also do not bound a backend title. The
source sites are [`pi/session_files.rs`](../src/modules/agents/adapter/pi/session_files.rs),
[`codex/catalog.rs`](../src/modules/agents/adapter/codex/catalog.rs),
[`opencode/catalog.rs`](../src/modules/agents/adapter/opencode/catalog.rs),
[`cursor/catalog.rs`](../src/modules/agents/adapter/cursor/catalog.rs), and
[`acp/backend.rs`](../src/modules/agents/adapter/acp/backend.rs).

Long titles are a separate navigation risk. The later snapshot found:

| Backend | Sessions | Titles over 120 chars | Longest title |
| --- | ---: | ---: | ---: |
| Pi | 1,019 | 11 | 225 |
| Codex | 376 | 191 | 189,561 |
| OpenCode | 125 | 0 | 81 |
| Cursor | 14 | 0 | 22 |
| Claude | 1 | 0 | 8 |

For all 376 Farcaster-indexed Codex sessions, the persisted title matched the
native field selected by the adapter's `name`, then `title`, then `preview`
precedence. All 191 titles over 120 characters came from native `title`; none
came from `name` or preview fallback. This rules out preview fallback as the
cause, but does not establish why Codex supplied long native titles. Since the
picker presents `session.title`, a 189 KB native title can make navigation and
layout work needlessly expensive.

Reproduce the aggregate payload and title measurements:

```sh
FARCASTER_DB=/Users/behzad/.local/share/farcaster/state.sqlite3
sqlite3 -readonly "$FARCASTER_DB" <<'SQL'
SELECT count(*) AS sessions,
       sum(CASE WHEN archived_at IS NULL THEN 1 ELSE 0 END) AS active,
       sum(CASE WHEN archived_at IS NOT NULL THEN 1 ELSE 0 END) AS archived,
       sum(length(search_text)) AS search_chars,
       sum(length(first_user_message)) AS first_message_chars,
       sum(length(title)) AS title_chars
  FROM sessions;

SELECT harness, count(*) AS sessions,
       sum(CASE WHEN length(title) > 120 THEN 1 ELSE 0 END) AS titles_over_120,
       max(length(title)) AS max_title_chars
  FROM sessions
 GROUP BY harness
 ORDER BY sessions DESC;

EXPLAIN QUERY PLAN
SELECT s.id, s.locator, p.path, s.title, s.first_user_message, s.timestamp,
       COALESCE(parent.backend_id, parent.locator, s.parent_backend_id),
       s.modified_ms, s.message_count, s.input_tokens, s.output_tokens,
       s.cache_read_tokens, s.cache_write_tokens, s.total_tokens,
       s.cost_micros, s.search_text, s.archived_at IS NOT NULL, s.harness,
       m.provider, m.model, m.effort, COALESCE(s.backend_id, s.locator)
  FROM sessions s
  JOIN projects p ON p.id = s.project_id
  LEFT JOIN sessions parent ON parent.id = s.parent_id
  LEFT JOIN session_models m ON m.session_id = s.id
 WHERE s.locator IS NOT NULL
 ORDER BY s.modified_ms DESC, s.timestamp DESC;
SQL
```

The Codex-source attribution query attaches both databases in read-only URI
mode and reports counts only:

```sh
sqlite3 -readonly 'file:/Users/behzad/.local/share/farcaster/state.sqlite3?mode=ro' <<'SQL'
ATTACH DATABASE 'file:/Users/behzad/.codex/state_5.sqlite?mode=ro' AS codex;
WITH resolved AS (
  SELECT s.title AS persisted_title,
         CASE
           WHEN trim(coalesce(t.name, '')) != '' THEN 'name'
           WHEN trim(coalesce(t.title, '')) != '' THEN 'title'
           WHEN trim(coalesce(t.preview, '')) != '' THEN 'preview'
           ELSE 'fallback'
         END AS selected_source,
         CASE
           WHEN trim(coalesce(t.name, '')) != '' THEN t.name
           WHEN trim(coalesce(t.title, '')) != '' THEN t.title
           WHEN trim(coalesce(t.preview, '')) != '' THEN t.preview
           ELSE 'New Codex session'
         END AS selected_value
    FROM sessions s
    JOIN codex.threads t ON s.harness = 'codex-cli'
                        AND s.locator LIKE '%/' || t.id
)
SELECT selected_source, count(*) AS sessions,
       sum(CASE WHEN length(persisted_title) > 120 THEN 1 ELSE 0 END)
         AS persisted_over_120,
       sum(CASE WHEN persisted_title = selected_value THEN 1 ELSE 0 END)
         AS same_as_selected
  FROM resolved
 GROUP BY selected_source
 ORDER BY selected_source;
DETACH DATABASE codex;
SQL
```

## Pi corpus bounds

The corrected per-file scan found 1,214 Pi JSONL files occupying 1.49 GB, with
287,871 total lines, 237.1 lines per file on average, a maximum of 4,521, and
52 files at or above 1,000 lines. None reached the 10,000-line adapter cap.
This does not show a current history truncation, though it does show enough
corpus size for repeated discovery and full-text catalog copies to matter.

Reproduce without printing file names or content:

```sh
find /Users/behzad/.pi/agent/sessions -type f -name '*.jsonl' \
  -exec sh -c 'for file do wc -l < "$file"; done' sh {} + 2>/dev/null |
  awk '{n++; total+=$1; if($1>max)max=$1; if($1>=1000)over_1k++}
       END {printf "files=%d total_lines=%d avg_lines=%.1f max_lines=%d over_1k=%d\\n",
                   n,total,(n ? total/n : 0),max,over_1k}'
du -sk /Users/behzad/.pi/agent/sessions
```

The Pi discovery limits are source-defined in
[`session_files.rs`](../src/modules/agents/adapter/pi/session_files.rs): 2,000
candidates, 2,000 directories, depth 6, 10,000 lines per file, and 64 KiB of
search text. The current corpus sits below the per-file line cap, so changing
that cap is not supported by this snapshot.
