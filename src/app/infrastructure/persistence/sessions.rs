use super::*;

impl StateStore {
    pub(crate) fn cached_sessions(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.cached_sessions");
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.id, s.locator, p.path, s.title, s.first_user_message, s.timestamp,
                        COALESCE(parent.backend_id, parent.locator, s.parent_backend_id),
                        s.modified_ms, s.message_count, s.input_tokens,
                        s.output_tokens, s.cache_read_tokens, s.cache_write_tokens,
                        s.total_tokens, s.cost_micros, s.search_text,
                        s.archived_at IS NOT NULL, s.harness,
                        m.provider, m.model, m.effort, COALESCE(s.backend_id, s.locator)
                   FROM sessions s
                   JOIN projects p ON p.id = s.project_id
                   LEFT JOIN sessions parent ON parent.id = s.parent_id
                   LEFT JOIN session_models m ON m.session_id = s.id
                  WHERE s.locator IS NOT NULL
                  ORDER BY s.modified_ms DESC, s.timestamp DESC",
            )
            .map_err(|error| format!("prepare cached sessions: {error}"))?;
        let rows = statement
            .query_map([], row_to_session)
            .map_err(|error| format!("query cached sessions: {error}"))?;
        let sessions = rows
            .map(|row| row.map_err(|error| format!("decode cached session: {error}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(crate::sessions::filter_session_tree(sessions, query))
    }

    #[cfg(test)]
    pub(crate) fn replace_sessions(&mut self, sessions: &[SessionSummary]) -> Result<(), String> {
        self.index_sessions(sessions, true)
    }

    pub(crate) fn index_sessions(
        &mut self,
        sessions: &[SessionSummary],
        prune_missing: bool,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session index update: {error}"))?;
        let known = sessions
            .iter()
            .map(|session| {
                (
                    session.harness.clone(),
                    crate::sessions::normalize_session_path(&session.path)
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .collect::<HashSet<_>>();
        for session in sessions {
            upsert_bound_session(&transaction, session)?;
        }
        transaction
            .execute_batch(
                "UPDATE sessions AS child SET parent_id=COALESCE(
               (SELECT parent.id FROM sessions parent
                 WHERE parent.harness=child.harness
                   AND (parent.backend_id=child.parent_backend_id
                        OR parent.locator=child.parent_backend_id)
                   AND parent.id != child.id LIMIT 1), child.parent_id)
             WHERE child.parent_backend_id IS NOT NULL;",
            )
            .map_err(|error| format!("resolve session parents: {error}"))?;
        if prune_missing {
            let candidates = transaction
                .prepare(
                    "SELECT id, harness, locator FROM sessions s
                      WHERE locator IS NOT NULL AND client_key IS NULL AND archived_at IS NULL
                        AND NOT EXISTS(SELECT 1 FROM composer_sessions WHERE session_id=s.id)
                        AND NOT EXISTS(SELECT 1 FROM outbox WHERE session_id=s.id)
                        AND NOT EXISTS(SELECT 1 FROM session_events WHERE session_id=s.id)
                        AND NOT EXISTS(SELECT 1 FROM session_ops WHERE session_id=s.id)
                        AND NOT EXISTS(SELECT 1 FROM sessions child WHERE child.parent_id=s.id)
                        AND NOT EXISTS(SELECT 1 FROM worker_families WHERE child_id=s.id)",
                )
                .map_err(|error| format!("read indexed locators: {error}"))?
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| format!("query indexed locators: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("decode indexed locators: {error}"))?;
            for (id, harness, locator) in candidates {
                if known.contains(&(harness, locator)) {
                    continue;
                }
                transaction
                    .execute("DELETE FROM sessions WHERE id=?1", [id])
                    .map_err(|error| format!("remove stale session: {error}"))?;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session index: {error}"))
    }

    pub(crate) fn has_queued_prompts_for(&self, paths: &[PathBuf]) -> Result<bool, String> {
        for path in paths {
            let locator = crate::sessions::normalize_session_path(path);
            let queued = self
                .connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM outbox o
                       JOIN sessions s ON s.id = o.session_id
                      WHERE s.locator=?1 AND o.state='queued'
                     )",
                    [locator.to_string_lossy()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| format!("check queued prompts for {}: {error}", path.display()))?;
            if queued {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn relocate_session_paths(
        &mut self,
        paths: &[(PathBuf, PathBuf)],
        target_project: &Path,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session path relocation: {error}"))?;
        let project_id = ensure_project(&transaction, target_project, u64_to_i64(now_ms()))?;
        for (source, target) in paths {
            let source_text = crate::sessions::normalize_session_path(source);
            let target_text = crate::sessions::normalize_session_path(target);
            transaction
                .execute(
                    "UPDATE sessions SET locator=?2, project_id=?3 WHERE locator=?1",
                    params![
                        source_text.to_string_lossy(),
                        target_text.to_string_lossy(),
                        project_id
                    ],
                )
                .map_err(|error| {
                    format!(
                        "relocate session state {} to {}: {error}",
                        source.display(),
                        target.display()
                    )
                })?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session path relocation: {error}"))
    }

    pub(crate) fn delete_session_state(&mut self, paths: &[PathBuf]) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session state deletion: {error}"))?;
        for path in paths {
            let locator = crate::sessions::normalize_session_path(path);
            transaction
                .execute(
                    "DELETE FROM sessions WHERE locator=?1",
                    [locator.to_string_lossy()],
                )
                .map_err(|error| format!("delete saved state for {}: {error}", path.display()))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session state deletion: {error}"))
    }

    pub(crate) fn set_session_archived(&self, path: &Path, archived: bool) -> Result<(), String> {
        let locator = crate::sessions::normalize_session_path(path);
        self.connection
            .execute(
                "UPDATE sessions SET archived_at=?2 WHERE locator=?1",
                params![
                    locator.to_string_lossy(),
                    archived.then_some(now_ms()).map(u64_to_i64)
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("update archived state for {}: {error}", path.display()))
    }
}

fn upsert_bound_session(
    transaction: &Transaction<'_>,
    session: &SessionSummary,
) -> Result<(), String> {
    let locator = crate::sessions::normalize_session_path(&session.path);
    let locator_text = locator.to_string_lossy();
    let project_id = ensure_project(
        transaction,
        &session.project,
        u64_to_i64(system_time_ms(session.modified)),
    )?;
    let archived = session
        .archived
        .then_some(system_time_ms(session.modified))
        .map(u64_to_i64);
    let existing = transaction
        .query_row(
            "SELECT id FROM sessions WHERE harness=?1 AND
               (locator=?2 OR (backend_id=?3 AND project_id=?4))
             ORDER BY locator=?2 DESC LIMIT 1",
            params![
                session.harness,
                locator_text.as_ref(),
                session.id,
                project_id
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("find session {}: {error}", session.path.display()))?;
    let existing = existing.or_else(|| {
        (session.app_session_id > 0)
            .then_some(session.app_session_id)
            .and_then(|id| {
                transaction
                    .query_row("SELECT id FROM sessions WHERE id=?1", [id], |row| {
                        row.get::<_, i64>(0)
                    })
                    .optional()
                    .ok()
                    .flatten()
            })
    });
    let id = if let Some(id) = existing {
        transaction
            .execute(
                "UPDATE sessions SET
                   project_id=?2, harness=?3, locator=?4, backend_id=?5, title=?6,
                   first_user_message=?7, search_text=?8, timestamp=?9, modified_ms=?10,
                   archived_at=COALESCE(archived_at, ?11), message_count=?12,
                   input_tokens=?13, output_tokens=?14, cache_read_tokens=?15,
                   cache_write_tokens=?16, total_tokens=?17, cost_micros=?18
                 WHERE id=?1",
                params![
                    id,
                    project_id,
                    session.harness,
                    locator_text.as_ref(),
                    session.id,
                    session.title,
                    session.first_user_message,
                    session.search_text(),
                    session.timestamp,
                    u64_to_i64(system_time_ms(session.modified)),
                    archived,
                    usize_to_u64(session.message_count),
                    session.usage.input,
                    session.usage.output,
                    session.usage.cache_read,
                    session.usage.cache_write,
                    session.usage.total,
                    session.usage.cost_micros,
                ],
            )
            .map_err(|error| format!("update session {}: {error}", session.path.display()))?;
        id
    } else {
        transaction
            .execute(
                "INSERT INTO sessions(
                   project_id, harness, locator, backend_id, title, first_user_message,
                   search_text, timestamp, modified_ms, archived_at, record_coverage,
                   message_count, input_tokens, output_tokens, cache_read_tokens,
                   cache_write_tokens, total_tokens, cost_micros, created_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'unloaded', ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?9)",
                params![
                    project_id,
                    session.harness,
                    locator_text.as_ref(),
                    session.id,
                    session.title,
                    session.first_user_message,
                    session.search_text(),
                    session.timestamp,
                    u64_to_i64(system_time_ms(session.modified)),
                    archived,
                    usize_to_u64(session.message_count),
                    session.usage.input,
                    session.usage.output,
                    session.usage.cache_read,
                    session.usage.cache_write,
                    session.usage.total,
                    session.usage.cost_micros,
                ],
            )
            .map_err(|error| format!("insert session {}: {error}", session.path.display()))?;
        transaction.last_insert_rowid()
    };
    transaction
        .execute(
            "UPDATE sessions SET backend_id=?2, parent_backend_id=?3 WHERE id=?1",
            params![id, session.id, session.parent_session],
        )
        .map_err(|error| format!("save backend session identity: {error}"))?;
    if let Some((provider, model)) = &session.model {
        transaction
            .execute(
                "INSERT INTO session_models(session_id, provider, model, effort)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(session_id) DO UPDATE SET
                   provider=excluded.provider, model=excluded.model, effort=excluded.effort",
                params![id, provider, model, session.thinking_level],
            )
            .map_err(|error| format!("save session model {}: {error}", session.path.display()))?;
    }
    Ok(())
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    let id = row.get::<_, i64>(0)?;
    let locator = row.get::<_, String>(1)?;
    let provider = row.get::<_, Option<String>>(18)?;
    let model = row.get::<_, Option<String>>(19)?;
    let effort = row.get::<_, Option<String>>(20)?;
    let mut session = SessionSummary::from_cached_for_harness(
        row.get(21)?,
        row.get(17)?,
        PathBuf::from(locator),
        PathBuf::from(row.get::<_, String>(2)?),
        row.get(3)?,
        row.get(4)?,
        row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        None,
        UNIX_EPOCH + std::time::Duration::from_millis(row.get::<_, u64>(7)?),
        row.get::<_, u64>(8)?.try_into().unwrap_or(usize::MAX),
        UsageSummary {
            input: row.get(9)?,
            output: row.get(10)?,
            cache_read: row.get(11)?,
            cache_write: row.get(12)?,
            total: row.get(13)?,
            cost_micros: row.get(14)?,
        },
        row.get(16)?,
        false,
        row.get(15)?,
    )
    .with_app_session_id(id);
    session.parent_session = row.get(6)?;
    if let (Some(provider), Some(model)) = (provider, model) {
        session.model = Some((provider, model));
        session.thinking_level = effort;
    }
    Ok(session)
}
