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

pub(super) fn merge_session(tx: &Transaction<'_>, keep: i64, other: i64) -> Result<(), String> {
    let offset: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM session_events WHERE session_id=?1",
            [keep],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let keep_client_key = tx
        .query_row(
            "SELECT client_key FROM sessions WHERE id=?1",
            [keep],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|error| error.to_string())?;
    let transferred_draft = if keep_client_key.is_none() {
        let other_draft = tx
            .query_row(
                "SELECT client_key,submitted,rail_order,created_ms FROM sessions WHERE id=?1",
                [other],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?;
        if other_draft.0.is_some() {
            tx.execute("UPDATE sessions SET client_key=NULL WHERE id=?1", [other])
                .map_err(|error| format!("release merged draft identity: {error}"))?;
            Some(other_draft)
        } else {
            None
        }
    } else {
        None
    };
    for sql in [
        "UPDATE sessions SET
           backend_id=COALESCE((SELECT backend_id FROM sessions WHERE id=?2),backend_id),
           parent_backend_id=COALESCE(parent_backend_id,(SELECT parent_backend_id FROM sessions WHERE id=?2)),
           title=COALESCE(NULLIF((SELECT title FROM sessions WHERE id=?2),''),title),
           first_user_message=COALESCE(NULLIF((SELECT first_user_message FROM sessions WHERE id=?2),''),first_user_message),
           search_text=TRIM(search_text || ' ' || (SELECT search_text FROM sessions WHERE id=?2)),
           timestamp=COALESCE((SELECT timestamp FROM sessions WHERE id=?2),timestamp),
           modified_ms=MAX(modified_ms,(SELECT modified_ms FROM sessions WHERE id=?2)),
           archived_at=COALESCE(archived_at,(SELECT archived_at FROM sessions WHERE id=?2)),
           record_coverage=CASE
             WHEN record_coverage='complete' OR (SELECT record_coverage FROM sessions WHERE id=?2)='complete' THEN 'complete'
             WHEN record_coverage='partial' OR (SELECT record_coverage FROM sessions WHERE id=?2)='partial' THEN 'partial'
             ELSE 'unloaded' END,
           message_count=MAX(message_count,(SELECT message_count FROM sessions WHERE id=?2)),
           input_tokens=MAX(input_tokens,(SELECT input_tokens FROM sessions WHERE id=?2)),
           output_tokens=MAX(output_tokens,(SELECT output_tokens FROM sessions WHERE id=?2)),
           cache_read_tokens=MAX(cache_read_tokens,(SELECT cache_read_tokens FROM sessions WHERE id=?2)),
           cache_write_tokens=MAX(cache_write_tokens,(SELECT cache_write_tokens FROM sessions WHERE id=?2)),
           total_tokens=MAX(total_tokens,(SELECT total_tokens FROM sessions WHERE id=?2)),
           cost_micros=MAX(cost_micros,(SELECT cost_micros FROM sessions WHERE id=?2)),
           submitted=MAX(submitted,(SELECT submitted FROM sessions WHERE id=?2)),
           rail_order=CASE WHEN rail_order=0
             THEN (SELECT rail_order FROM sessions WHERE id=?2) ELSE rail_order END,
           created_ms=MIN(created_ms,(SELECT created_ms FROM sessions WHERE id=?2)),
           parent_id=COALESCE(parent_id,(SELECT parent_id FROM sessions WHERE id=?2))
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
    if let Some((client_key, submitted, rail_order, created_ms)) = transferred_draft {
        tx.execute(
            "UPDATE sessions SET client_key=?2,submitted=?3,rail_order=?4,created_ms=?5 WHERE id=?1",
            params![keep, client_key, submitted, rail_order, created_ms],
        )
        .map_err(|error| format!("retain merged draft state: {error}"))?;
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

pub(super) fn family_locator_root(locator_root: &Path, project: &Path) -> PathBuf {
    use sha2::{Digest as _, Sha256};

    let project = crate::sessions::normalize_session_path(project);
    let digest = Sha256::digest(project.to_string_lossy().as_bytes());
    locator_root.join(format!("{digest:x}"))
}
pub(super) fn ensure_locator_session(
    transaction: &Transaction<'_>,
    harness: &str,
    identity: &str,
    project_id: i64,
    locator_root: &Path,
) -> Result<i64, String> {
    let supplied_path = Path::new(identity);
    let native_id = (!supplied_path.is_absolute()).then_some(identity);
    let locator = if supplied_path.is_absolute() {
        crate::sessions::normalize_session_path(supplied_path)
    } else {
        let encoded = url::form_urlencoded::byte_serialize(identity.as_bytes()).collect::<String>();
        locator_root.join(harness).join(encoded)
    };
    let locator_text = locator.to_string_lossy();
    let mut statement = transaction
        .prepare(
            "SELECT id, locator FROM sessions WHERE harness=?1 AND project_id=?2
               AND (locator=?3 OR (?4 IS NOT NULL AND backend_id=?4))
             ORDER BY backend_id=?4 DESC, locator=?3 DESC, id",
        )
        .map_err(|error| format!("prepare family session lookup: {error}"))?;
    let ids = statement
        .query_map(
            params![harness, project_id, locator_text.as_ref(), native_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .map_err(|error| format!("find family session: {error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("decode family session: {error}"))?;
    drop(statement);
    if let Some(((keep, keep_locator), others)) = ids.split_first() {
        for &(other, _) in others {
            merge_session(transaction, *keep, other)?;
        }
        let retained_locator = keep_locator.as_deref().filter(|current| {
            native_id.is_some_and(|native_id| {
                crate::agents::external_session_identity(Path::new(current)).is_some_and(
                    |(stored_harness, stored_id)| {
                        stored_harness == harness && stored_id == native_id
                    },
                )
            })
        });
        transaction
            .execute(
                "UPDATE sessions SET locator=?2, backend_id=COALESCE(?3,backend_id) WHERE id=?1",
                params![
                    *keep,
                    retained_locator.unwrap_or(locator_text.as_ref()),
                    native_id
                ],
            )
            .map_err(|error| format!("canonicalize family session: {error}"))?;
        return Ok(*keep);
    }
    let now = u64_to_i64(now_ms());
    transaction
        .execute(
            "INSERT INTO sessions(
               project_id, harness, locator, backend_id, modified_ms, created_ms
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?5)",
            params![project_id, harness, locator_text.as_ref(), native_id, now],
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
            legacy_session_ids_for_locator(&self.connection, &locator, harness, None)?
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
    project: Option<&Path>,
) -> Result<Vec<i64>, String> {
    let mut statement = connection
        .prepare(
            "SELECT s.id, s.locator, p.path FROM sessions s
              JOIN projects p ON p.id=s.project_id
             WHERE s.locator IS NOT NULL AND (?1 IS NULL OR s.harness=?1)",
        )
        .map_err(|error| format!("prepare legacy session lookup: {error}"))?;
    statement
        .query_map([harness], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("query legacy session lookup: {error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("decode legacy session lookup: {error}"))
        .map(|rows| {
            rows.into_iter()
                .filter_map(|(id, candidate, candidate_project)| {
                    let locator_matches =
                        crate::sessions::normalize_session_path(Path::new(&candidate)) == locator;
                    let project_matches = project.is_none_or(|project| {
                        crate::sessions::normalize_session_path(Path::new(&candidate_project))
                            == crate::sessions::normalize_session_path(project)
                    });
                    (locator_matches && project_matches).then_some(id)
                })
                .collect()
        })
}

pub(super) fn legacy_session_id_for_locator(
    transaction: &Transaction<'_>,
    harness: &str,
    locator: &Path,
    project: &Path,
) -> Result<Option<i64>, String> {
    let ids = legacy_session_ids_for_locator(transaction, locator, Some(harness), Some(project))?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => Err(format!(
            "legacy session locator is ambiguous for {harness}: {}",
            locator.display()
        )),
    }
}
