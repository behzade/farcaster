use super::*;

pub(super) fn bind_locator(
    transaction: &Transaction<'_>,
    draft_id: &str,
    locator: &Path,
) -> Result<(), String> {
    let locator = locator.to_string_lossy();
    let draft_row: Option<(i64, String)> = transaction
        .query_row(
            "SELECT id, harness FROM sessions WHERE client_key=?1",
            [draft_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((draft_session_id, harness)) = draft_row else {
        return Ok(());
    };
    let existing: Option<i64> = transaction
        .query_row(
            "SELECT id FROM sessions WHERE harness=?1 AND locator=?2",
            params![harness, locator.as_ref()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(existing) = existing.filter(|existing| *existing != draft_session_id) {
        merge_session(transaction, draft_session_id, existing)?;
    }
    transaction
        .execute(
            "UPDATE sessions SET locator=?2 WHERE id=?1",
            params![draft_session_id, locator.as_ref()],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn merge_session(tx: &Transaction<'_>, keep: i64, other: i64) -> Result<(), String> {
    let offset: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM session_events WHERE session_id=?1",
            [keep],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    for sql in [
        "UPDATE sessions SET
           (backend_id, parent_backend_id, title, first_user_message, search_text,
            timestamp, modified_ms, archived_at, record_coverage, message_count,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
            total_tokens, cost_micros) =
           (SELECT backend_id, parent_backend_id, title, first_user_message, search_text,
            timestamp, modified_ms, archived_at, record_coverage, message_count,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
            total_tokens, cost_micros FROM sessions WHERE id=?2),
           parent_id=COALESCE(parent_id, (SELECT parent_id FROM sessions WHERE id=?2))
         WHERE id=?1",
        "INSERT INTO composer_sessions(session_id, text, cursor, selection_start, selection_end, history_json, updated_ms, attachments_json)
         SELECT ?1, text, cursor, selection_start, selection_end, history_json, updated_ms, attachments_json
           FROM composer_sessions WHERE session_id=?2
         ON CONFLICT(session_id) DO UPDATE SET
           text=excluded.text, cursor=excluded.cursor, selection_start=excluded.selection_start,
           selection_end=excluded.selection_end, history_json=excluded.history_json,
           attachments_json=excluded.attachments_json,
           updated_ms=excluded.updated_ms
         WHERE excluded.updated_ms > composer_sessions.updated_ms",
        "INSERT INTO session_models SELECT ?1, provider, model, effort, service_tier
           FROM session_models WHERE session_id=?2
         ON CONFLICT(session_id) DO NOTHING",
        "INSERT INTO worker_families SELECT ?1, execution_json FROM worker_families WHERE child_id=?2
         ON CONFLICT(child_id) DO NOTHING",
        "UPDATE session_ops SET session_id=?1 WHERE session_id=?2",
        "UPDATE sessions SET parent_id=?1 WHERE parent_id=?2 AND id != ?1",
    ] {
        tx.execute(sql, params![keep, other]).map_err(|error| format!("merge session state: {error}"))?;
    }
    tx.execute(
        "INSERT INTO session_events SELECT ?1, seq+?3, t, schema_version, body
           FROM session_events WHERE session_id=?2",
        params![keep, other, offset],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE outbox SET session_id=?1, submission_event_seq=submission_event_seq+?3
          WHERE session_id=?2",
        params![keep, other, offset],
    )
    .map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM sessions WHERE id=?1", [other])
        .map_err(|error| error.to_string())?;
    Ok(())
}
pub(super) fn ensure_locator_session(
    transaction: &Transaction<'_>,
    harness: &str,
    locator: &str,
    project_id: i64,
) -> Result<i64, String> {
    if let Some(id) = transaction
        .query_row(
            "SELECT id FROM sessions WHERE harness=?1 AND project_id=?3
               AND (locator=?2 OR backend_id=?2) ORDER BY locator=?2 DESC LIMIT 1",
            params![harness, locator, project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("find locator session: {error}"))?
    {
        return Ok(id);
    }
    let now = u64_to_i64(now_ms());
    transaction
        .execute(
            "INSERT INTO sessions(
               project_id, harness, locator, backend_id, modified_ms, created_ms
             ) VALUES(?1, ?2, ?3, ?3, ?4, ?4)",
            params![project_id, harness, locator, now],
        )
        .map_err(|error| format!("insert locator session: {error}"))?;
    Ok(transaction.last_insert_rowid())
}

pub(super) fn ensure_project(
    transaction: &Transaction<'_>,
    path: &Path,
    added_ms: i64,
) -> Result<i64, String> {
    let path = crate::sessions::normalize_session_path(path);
    let path = path.to_string_lossy();
    transaction
        .execute(
            "INSERT INTO projects(path, added_ms) VALUES(?1, ?2)
             ON CONFLICT(path) DO NOTHING",
            params![path.as_ref(), added_ms],
        )
        .map_err(|error| format!("ensure project {path}: {error}"))?;
    transaction
        .query_row(
            "SELECT id FROM projects WHERE path=?1",
            [path.as_ref()],
            |row| row.get(0),
        )
        .map_err(|error| format!("read project {path}: {error}"))
}

pub(super) fn target_for_session(
    client_key: Option<&str>,
    locator: Option<&str>,
) -> rusqlite::Result<String> {
    if let Some(locator) = locator {
        Ok(format!(
            "session:{}",
            crate::sessions::normalize_session_path(Path::new(locator)).display()
        ))
    } else if let Some(key) = client_key {
        Ok(format!("draft:{key}"))
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn target_locator(target: &str, session_path: Option<&Path>) -> Option<PathBuf> {
    session_path
        .map(PathBuf::from)
        .or_else(|| {
            target
                .strip_prefix("session:")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .map(|path| crate::sessions::normalize_session_path(&path))
}

pub(super) fn create_target_session(
    tx: &Transaction<'_>,
    target: &str,
    harness: &str,
    project: &Path,
    session_path: Option<&Path>,
) -> Result<i64, String> {
    let client_key = target.strip_prefix("draft:").filter(|key| !key.is_empty());
    let locator = target_locator(target, session_path);
    if client_key.is_none() && locator.is_none() {
        return Err(format!("invalid session target: {target}"));
    }
    let now = u64_to_i64(now_ms());
    let project_id = ensure_project(tx, project, now)?;
    tx.execute(
        "INSERT INTO sessions(project_id,harness,locator,client_key,modified_ms,created_ms)
         VALUES(?1,?2,?3,?4,?5,?5)",
        params![
            project_id,
            harness,
            locator.as_ref().map(|path| path.to_string_lossy()),
            client_key,
            now
        ],
    )
    .map_err(|error| format!("create prompt session: {error}"))?;
    Ok(tx.last_insert_rowid())
}

impl StateStore {
    pub(super) fn session_id_for_target(
        &self,
        target: &str,
        session_path: Option<&Path>,
        harness: Option<&str>,
    ) -> Result<Option<i64>, String> {
        if let Some(key) = target.strip_prefix("draft:") {
            let id = self
                .connection
                .query_row(
                    "SELECT id FROM sessions WHERE client_key=?1 AND (?2 IS NULL OR harness=?2)",
                    params![key, harness],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| format!("resolve draft {key}: {error}"))?;
            if id.is_some() {
                return Ok(id);
            }
        }
        let locator = target_locator(target, session_path);
        let Some(locator) = locator else {
            return Ok(None);
        };
        let mut statement = self
            .connection
            .prepare(
                "SELECT id FROM sessions WHERE locator=?1 AND (?2 IS NULL OR harness=?2) LIMIT 2",
            )
            .map_err(|error| format!("resolve session: {error}"))?;
        let ids = statement
            .query_map(params![locator.to_string_lossy(), harness], |row| {
                row.get(0)
            })
            .map_err(|error| format!("resolve session: {error}"))?
            .collect::<rusqlite::Result<Vec<i64>>>()
            .map_err(|error| format!("resolve session: {error}"))?;
        let ids = if ids.is_empty() {
            legacy_session_ids_for_locator(&self.connection, &locator, harness)?
        } else {
            ids
        };
        match ids.as_slice() {
            [] => Ok(None),
            [id] => Ok(Some(*id)),
            _ => Err(format!(
                "session locator is ambiguous across harnesses: {}",
                locator.display()
            )),
        }
    }
}

fn legacy_session_ids_for_locator(
    connection: &Connection,
    locator: &Path,
    harness: Option<&str>,
) -> Result<Vec<i64>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, locator FROM sessions
              WHERE locator IS NOT NULL AND (?1 IS NULL OR harness=?1)",
        )
        .map_err(|error| format!("prepare legacy session lookup: {error}"))?;
    statement
        .query_map([harness], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query legacy session lookup: {error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("decode legacy session lookup: {error}"))
        .map(|rows| {
            rows.into_iter()
                .filter_map(|(id, candidate)| {
                    (crate::sessions::normalize_session_path(Path::new(&candidate)) == locator)
                        .then_some(id)
                })
                .collect()
        })
}

pub(super) fn legacy_session_id_for_locator(
    transaction: &Transaction<'_>,
    harness: &str,
    locator: &Path,
) -> Result<Option<i64>, String> {
    let ids = legacy_session_ids_for_locator(transaction, locator, Some(harness))?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => Err(format!(
            "legacy session locator is ambiguous for {harness}: {}",
            locator.display()
        )),
    }
}
