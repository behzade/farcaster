use std::collections::HashMap;

use super::*;

pub(super) fn migrate_v11_to_v12(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "DROP INDEX IF EXISTS sessions_parent;
         DROP INDEX IF EXISTS sessions_modified;
         ALTER TABLE projects RENAME TO projects_legacy;
         ALTER TABLE sessions RENAME TO sessions_legacy;
         ALTER TABLE outbox RENAME TO outbox_legacy;
         ALTER TABLE composer_sessions RENAME TO composer_sessions_legacy;",
    )
    .map_err(|error| format!("rename legacy catalog tables: {error}"))?;
    tx.execute_batch(include_str!("schema.sql"))
        .map_err(|error| format!("create v12 tables: {error}"))?;
    copy_projects(tx)?;
    copy_sessions(tx)?;
    tx.execute_batch(
        "UPDATE sessions SET
           backend_id=(SELECT id FROM sessions_legacy WHERE path=sessions.locator),
           parent_backend_id=(SELECT parent_session FROM sessions_legacy WHERE path=sessions.locator),
           submitted=COALESCE((SELECT submitted FROM drafts WHERE id=sessions.client_key), 0);
         UPDATE sessions AS child SET parent_id=(
           SELECT parent.id FROM sessions parent
            WHERE parent.harness=child.harness AND parent.backend_id=child.parent_backend_id
              AND parent.id != child.id LIMIT 1
         ) WHERE child.parent_backend_id IS NOT NULL;"
    ).map_err(|error| format!("copy backend session identities: {error}"))?;
    copy_outbox(tx)?;
    copy_composer(tx)?;
    copy_presentations(tx)?;
    copy_ui_state(tx)?;
    copy_worker_families(tx)?;
    tx.execute_batch(
        "DROP TABLE IF EXISTS prompt_presentations;
         DROP TABLE IF EXISTS composer_sessions_legacy;
         DROP TABLE IF EXISTS outbox_legacy;
         DROP TABLE IF EXISTS drafts;
         DROP TABLE IF EXISTS app_sessions;
         DROP TABLE IF EXISTS sessions_legacy;
         DROP TABLE IF EXISTS projects_legacy;
         DELETE FROM meta WHERE key != 'schema_version';
",
    )
    .map_err(|error| format!("replace legacy tables for v12: {error}"))?;
    Ok(())
}

fn copy_projects(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute(
        "INSERT INTO projects(path, added_ms, deleted_at)
         SELECT path, added_ms, NULL FROM projects_legacy",
        [],
    )
    .map_err(|error| format!("copy projects: {error}"))?;
    if let Ok(excluded) = tx.query_row(
        "SELECT value FROM meta WHERE key='excluded_projects'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        let paths: Vec<String> = serde_json::from_str(&excluded).unwrap_or_default();
        let now = u64_to_i64(now_ms());
        for path in paths {
            tx.execute(
                "INSERT INTO projects(path, added_ms, deleted_at)
                 VALUES(?1, ?2, ?2)
                 ON CONFLICT(path) DO UPDATE SET deleted_at=excluded.deleted_at",
                params![path, now],
            )
            .map_err(|error| format!("copy excluded project: {error}"))?;
        }
    }
    if let Ok(prefs) = tx.query_row(
        "SELECT value FROM meta WHERE key='repository_backend_preferences'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        let prefs: BTreeMap<String, String> = serde_json::from_str(&prefs).unwrap_or_default();
        for (path, backend) in prefs {
            tx.execute(
                "UPDATE projects SET repository_backend=?2 WHERE path=?1",
                params![path, backend],
            )
            .map_err(|error| format!("copy repository preference: {error}"))?;
        }
    }
    Ok(())
}

fn copy_sessions(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute_batch(
        "INSERT INTO projects(path, added_ms)
           SELECT project, MIN(modified_ms) FROM sessions_legacy GROUP BY project
           ON CONFLICT(path) DO NOTHING;
         INSERT INTO projects(path, added_ms)
           SELECT project, MIN(created_ms) FROM drafts GROUP BY project
           ON CONFLICT(path) DO NOTHING;
         INSERT INTO projects(path, added_ms)
           SELECT 'unknown', MIN(a.created_ms) FROM app_sessions a
           LEFT JOIN sessions_legacy s ON s.path=a.session_path
           LEFT JOIN drafts d ON d.app_session_id=a.id
           WHERE s.project IS NULL AND d.project IS NULL HAVING COUNT(*) > 0
           ON CONFLICT(path) DO NOTHING;

         INSERT INTO sessions(
           id,project_id,harness,locator,client_key,title,first_user_message,search_text,
           timestamp,modified_ms,archived_at,message_count,input_tokens,output_tokens,
           cache_read_tokens,cache_write_tokens,total_tokens,cost_micros,created_ms,submitted)
         SELECT a.id,p.id,COALESCE(d.harness,s.harness,a.harness),
           NULLIF(COALESCE(a.session_path,d.session_path),''),
           COALESCE(a.draft_id,d.id),COALESCE(NULLIF(s.title,''),d.provisional_title,''),
           COALESCE(s.first_user_message,''),COALESCE(s.search_text,''),s.timestamp,
           COALESCE(s.modified_ms,a.created_ms),s.settled_ms,COALESCE(s.message_count,0),
           COALESCE(s.input_tokens,0),COALESCE(s.output_tokens,0),COALESCE(s.cache_read_tokens,0),
           COALESCE(s.cache_write_tokens,0),COALESCE(s.total_tokens,0),COALESCE(s.cost_micros,0),
           a.created_ms,COALESCE(d.submitted,0)
         FROM app_sessions a
         LEFT JOIN sessions_legacy s ON s.path=a.session_path
         LEFT JOIN drafts d ON d.app_session_id=a.id
         JOIN projects p ON p.path=COALESCE(d.project,s.project,'unknown')
         ORDER BY a.id;

         INSERT INTO sessions(
           project_id,harness,locator,title,first_user_message,search_text,timestamp,
           modified_ms,archived_at,message_count,input_tokens,output_tokens,cache_read_tokens,
           cache_write_tokens,total_tokens,cost_micros,created_ms)
         SELECT p.id,s.harness,s.path,s.title,s.first_user_message,s.search_text,s.timestamp,
           s.modified_ms,s.settled_ms,s.message_count,s.input_tokens,s.output_tokens,s.cache_read_tokens,
           s.cache_write_tokens,s.total_tokens,s.cost_micros,s.modified_ms
         FROM sessions_legacy s JOIN projects p ON p.path=s.project
         WHERE NOT EXISTS(SELECT 1 FROM sessions n WHERE n.harness=s.harness AND n.locator=s.path);

         INSERT INTO sessions(project_id,harness,locator,client_key,title,modified_ms,created_ms,submitted)
         SELECT p.id,d.harness,d.session_path,d.id,COALESCE(d.provisional_title,''),d.created_ms,d.created_ms,d.submitted
         FROM drafts d JOIN projects p ON p.path=d.project
         WHERE NOT EXISTS(SELECT 1 FROM sessions n WHERE n.client_key=d.id);

         UPDATE sessions SET rail_order=COALESCE((
           SELECT CAST(j.key AS INTEGER) FROM meta, json_each(meta.value) j
            WHERE meta.key='app_session_order' AND j.value=sessions.id),0);"
    ).map_err(|error| format!("copy legacy sessions: {error}"))
}

fn copy_outbox(tx: &Transaction<'_>) -> Result<(), String> {
    let mut statement = tx
        .prepare(
            "SELECT id, target, session_path, mode, message, display_message, invocation,
                    images_json, state, error, created_ms
               FROM outbox_legacy",
        )
        .map_err(|error| format!("read legacy outbox: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })
        .map_err(|error| format!("query legacy outbox: {error}"))?;
    for row in rows {
        let (
            id,
            target,
            session_path,
            mode,
            message,
            display,
            invocation,
            images,
            state,
            error,
            created,
        ) = row.map_err(|error| format!("decode outbox: {error}"))?;
        let Some(session_id) = resolve_target(tx, &target, session_path.as_deref())? else {
            continue;
        };
        let state = match state.as_str() {
            "sending" => "sending",
            "failed" => "failed",
            _ => "queued",
        };
        tx.execute(
            "INSERT INTO outbox(
               id, session_id, mode, message, display_message, invocation, images_json,
               state, error, created_ms
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id, session_id, mode, message, display, invocation, images, state, error, created
            ],
        )
        .map_err(|error| format!("copy outbox {id}: {error}"))?;
    }
    Ok(())
}

fn copy_composer(tx: &Transaction<'_>) -> Result<(), String> {
    let mut statement = tx
        .prepare(
            "SELECT target, text, cursor, selection_start, selection_end, history_json, updated_ms
               FROM composer_sessions_legacy",
        )
        .map_err(|error| format!("read legacy composer: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|error| format!("query legacy composer: {error}"))?;
    for row in rows {
        let (target, text, cursor, start, end, history, updated) =
            row.map_err(|error| format!("decode composer: {error}"))?;
        let Some(session_id) = resolve_target(tx, &target, None)? else {
            continue;
        };
        tx.execute(
            "INSERT OR REPLACE INTO composer_sessions(
               session_id, text, cursor, selection_start, selection_end, history_json, updated_ms
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![session_id, text, cursor, start, end, history, updated],
        )
        .map_err(|error| format!("copy composer {session_id}: {error}"))?;
    }
    Ok(())
}

fn copy_presentations(tx: &Transaction<'_>) -> Result<(), String> {
    let mut statement = tx
        .prepare(
            "SELECT session_path, resolved_message, display_message, invocation, created_ms
               FROM prompt_presentations ORDER BY created_ms, id",
        )
        .map_err(|error| format!("read presentations: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| format!("query presentations: {error}"))?;
    let mut seq_by_session = HashMap::<i64, i64>::new();
    for row in rows {
        let (path, resolved, display, invocation, created) =
            row.map_err(|error| format!("decode presentation: {error}"))?;
        let Some(session_id) = resolve_target(tx, "", Some(&path))? else {
            continue;
        };
        let seq = seq_by_session.entry(session_id).or_insert(0);
        *seq += 1;
        let body = serde_json::json!({
            "type": "prompt_presentation",
            "resolved": resolved,
            "display": display,
            "invocation": invocation,
        })
        .to_string();
        tx.execute(
            "INSERT INTO session_events(session_id, seq, t, schema_version, body)
             VALUES(?1, ?2, ?3, 1, ?4)",
            params![session_id, *seq, created, body],
        )
        .map_err(|error| format!("copy presentation: {error}"))?;
    }
    Ok(())
}

fn copy_ui_state(tx: &Transaction<'_>) -> Result<(), String> {
    let meta = |key: &str| {
        tx.query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .unwrap_or(None)
    };
    let mcp = match meta("builtin_mcp_enabled").as_deref() {
        Some("false") => 0,
        _ => 1,
    };
    tx.execute(
        "INSERT INTO ui_state(
           id, window_placement_json, network_proxy,
           builtin_mcp_enabled, worker_tasks_json, configuration_catalogs_json,
           session_control_defaults_json, app_session_order_json
         ) VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            meta("window_placement"),
            meta("network_proxy"),
            mcp,
            meta("worker_tasks"),
            meta("configuration_catalogs"),
            meta("session_control_defaults"),
            meta("app_session_order"),
        ],
    )
    .map_err(|error| format!("copy ui_state: {error}"))?;
    Ok(())
}

fn copy_worker_families(tx: &Transaction<'_>) -> Result<(), String> {
    let mut statement = tx
        .prepare("SELECT value FROM meta WHERE key GLOB 'worker_family:*'")
        .map_err(|error| format!("read worker families: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("query worker families: {error}"))?;
    for row in rows {
        let value = row.map_err(|error| format!("decode worker family: {error}"))?;
        let Ok(link) = serde_json::from_str::<crate::agents::WorkerFamilyLink>(&value) else {
            continue;
        };
        let Some(child_id) = resolve_target(tx, "", Some(&link.child_session))?.or_else(|| {
            resolve_target(tx, "", Some(&link.child_session))
                .ok()
                .flatten()
        }) else {
            continue;
        };
        let execution = serde_json::to_string(&link.execution).ok();
        tx.execute(
            "INSERT OR REPLACE INTO worker_families(child_id, execution_json) VALUES(?1, ?2)",
            params![child_id, execution],
        )
        .map_err(|error| format!("copy worker family: {error}"))?;
        if let Some(parent_id) = resolve_target(tx, "", Some(&link.parent_session))? {
            tx.execute(
                "UPDATE sessions SET parent_id=?2 WHERE id=?1 AND parent_id IS NULL",
                params![child_id, parent_id],
            )
            .map_err(|error| format!("copy worker parent: {error}"))?;
        }
    }
    Ok(())
}

fn resolve_target(
    tx: &Transaction<'_>,
    target: &str,
    session_path: Option<&str>,
) -> Result<Option<i64>, String> {
    if let Some(path) = session_path.filter(|path| !path.is_empty())
        && let Some(id) = tx
            .query_row("SELECT id FROM sessions WHERE locator=?1", [path], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| format!("resolve locator {path}: {error}"))?
    {
        return Ok(Some(id));
    }
    if let Some(draft) = target.strip_prefix("draft:") {
        return tx
            .query_row(
                "SELECT id FROM sessions WHERE client_key=?1",
                [draft],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("resolve draft {draft}: {error}"));
    }
    if let Some(path) = target.strip_prefix("session:") {
        return tx
            .query_row("SELECT id FROM sessions WHERE locator=?1", [path], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| format!("resolve session {path}: {error}"));
    }
    Ok(None)
}

pub(super) fn import_legacy_pi_gpui(tx: &Transaction<'_>) -> Result<(), String> {
    let normalized: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions', 'legacy_pi_gpui') WHERE name='locator')",
        [], |row| row.get(0),
    ).map_err(|error| format!("inspect legacy sessions: {error}"))?;
    let (projects, sessions, archives, composers) = if normalized {
        (
            "INSERT INTO projects(path, added_ms, deleted_at, repository_backend)
             SELECT path, added_ms, deleted_at, repository_backend FROM legacy_pi_gpui.projects WHERE true
             ON CONFLICT(path) DO NOTHING",
            "INSERT INTO sessions(project_id, harness, locator, title, first_user_message, search_text,
                 timestamp, modified_ms, archived_at, message_count, input_tokens, output_tokens,
                 cache_read_tokens, cache_write_tokens, total_tokens, cost_micros, created_ms)
             SELECT p.id, l.harness, l.locator, l.title, l.first_user_message, l.search_text,
                 l.timestamp, l.modified_ms, l.archived_at, l.message_count, l.input_tokens, l.output_tokens,
                 l.cache_read_tokens, l.cache_write_tokens, l.total_tokens, l.cost_micros, l.created_ms
             FROM legacy_pi_gpui.sessions l JOIN legacy_pi_gpui.projects lp ON lp.id=l.project_id
             JOIN projects p ON p.path=lp.path WHERE l.locator IS NOT NULL
             ON CONFLICT(harness, locator) DO NOTHING",
            "UPDATE sessions AS s SET archived_at=COALESCE(archived_at,
                (SELECT l.archived_at FROM legacy_pi_gpui.sessions l
                  WHERE l.harness=s.harness AND l.locator=s.locator))",
            "INSERT INTO composer_sessions(session_id, text, cursor, selection_start, selection_end, history_json, updated_ms)
             SELECT s.id, c.text, c.cursor, c.selection_start, c.selection_end, c.history_json, c.updated_ms
             FROM legacy_pi_gpui.composer_sessions c
             JOIN legacy_pi_gpui.sessions l ON l.id=c.session_id
             JOIN sessions s ON s.harness=l.harness AND s.locator=l.locator WHERE true
             ON CONFLICT(session_id) DO UPDATE SET text=excluded.text, cursor=excluded.cursor,
                 selection_start=excluded.selection_start, selection_end=excluded.selection_end,
                 history_json=excluded.history_json, updated_ms=excluded.updated_ms
             WHERE excluded.updated_ms > composer_sessions.updated_ms",
        )
    } else {
        (
            "INSERT INTO projects(path, added_ms)
             SELECT path, added_ms FROM legacy_pi_gpui.projects WHERE true
             ON CONFLICT(path) DO NOTHING",
            "INSERT INTO sessions(project_id, harness, locator, backend_id, parent_backend_id,
                 title, first_user_message, search_text, timestamp, modified_ms, archived_at,
                 message_count, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                 total_tokens, cost_micros, created_ms)
             SELECT p.id, 'pi', l.path, l.id, l.parent_session, l.title, l.first_user_message,
                 l.search_text, l.timestamp, l.modified_ms, l.settled_ms, l.message_count,
                 l.input_tokens, l.output_tokens, l.cache_read_tokens, l.cache_write_tokens,
                 l.total_tokens, l.cost_micros, l.modified_ms
             FROM legacy_pi_gpui.sessions l JOIN projects p ON p.path=l.project WHERE true
             ON CONFLICT(harness, locator) DO NOTHING",
            "UPDATE sessions AS s SET archived_at=COALESCE(archived_at,
                (SELECT l.settled_ms FROM legacy_pi_gpui.sessions l
                  WHERE s.harness='pi' AND l.path=s.locator))",
            "INSERT INTO composer_sessions(session_id, text, cursor, selection_start, selection_end, history_json, updated_ms)
             SELECT s.id, c.text, c.cursor, c.selection_start, c.selection_end, c.history_json, c.updated_ms
             FROM legacy_pi_gpui.composer_sessions c
             JOIN sessions s ON c.target='session:' || s.locator AND s.harness='pi' WHERE true
             ON CONFLICT(session_id) DO UPDATE SET text=excluded.text, cursor=excluded.cursor,
                 selection_start=excluded.selection_start, selection_end=excluded.selection_end,
                 history_json=excluded.history_json, updated_ms=excluded.updated_ms
             WHERE excluded.updated_ms > composer_sessions.updated_ms",
        )
    };
    for sql in [projects, sessions, archives, composers] {
        tx.execute(sql, [])
            .map_err(|error| format!("import legacy pi-gpui state: {error}"))?;
    }
    Ok(())
}
