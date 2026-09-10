use super::*;

impl StateStore {
    pub(crate) fn cached_sessions(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        Ok(crate::sessions::filter_session_tree(
            Self::read_cached_sessions(&self.connection, None)?,
            query,
        ))
    }

    fn read_cached_sessions(
        connection: &Connection,
        id: Option<i64>,
    ) -> Result<Vec<SessionSummary>, String> {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.cached_sessions");
        let mut statement = connection
            .prepare(&format!(
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
                  WHERE s.locator IS NOT NULL AND {}
                  ORDER BY s.modified_ms DESC, s.timestamp DESC",
                if id.is_some() {
                    "s.id=?1"
                } else {
                    "?1 IS NULL"
                }
            ))
            .map_err(|error| format!("prepare cached sessions: {error}"))?;
        let rows = statement
            .query_map([id], row_to_session)
            .map_err(|error| format!("query cached sessions: {error}"))?;
        let sessions = rows
            .map(|row| row.map_err(|error| format!("decode cached session: {error}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    pub(crate) fn update_session_metadata(
        &mut self,
        update: &crate::agents::SessionMetadata,
    ) -> Result<SessionSummary, String> {
        let path = crate::sessions::normalize_session_path(&update.path);
        let tx = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let now = u64_to_i64(now_ms());
        let project = ensure_project(&tx, &update.project, now)?;
        let existing = tx
            .query_row(
                "SELECT id FROM sessions WHERE harness=?1 AND locator=?2",
                params![update.harness, path.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let existing = if existing.is_some() {
            existing
        } else {
            super::identity::legacy_session_id_for_locator(&tx, &update.harness, &path)?
        };
        let id = if let Some(id) = existing {
            id
        } else {
            tx.execute(
                "INSERT INTO sessions(project_id,harness,locator,backend_id,modified_ms,created_ms)
                 VALUES(?1,?2,?3,?4,?5,?5)",
                params![
                    project,
                    update.harness,
                    path.to_string_lossy(),
                    update.id,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
            tx.last_insert_rowid()
        };
        tx.execute(
            "UPDATE sessions SET project_id=?2, locator=?3, backend_id=?4,
               search_text=CASE WHEN ?5 IS NOT NULL AND ?5 != title
                   THEN search_text || ' ' || LOWER(?5) ELSE search_text END,
               title=COALESCE(?5,NULLIF(title,''),?6,''),
               first_user_message=CASE WHEN first_user_message='' THEN COALESCE(?6,'') ELSE first_user_message END,
               parent_backend_id=COALESCE(?7,parent_backend_id),
               parent_id=COALESCE((SELECT id FROM sessions WHERE harness=?8 AND backend_id=?7 LIMIT 1),parent_id),
               message_count=COALESCE(?9,message_count), modified_ms=?10
             WHERE id=?1",
            params![id, project, path.to_string_lossy(), update.id, update.title,
                update.first_user_message, update.parent_session, update.harness,
                update.message_count.map(|n| n as i64), now],
        ).map_err(|error| format!("update live session metadata: {error}"))?;
        tx.execute(
            "UPDATE sessions SET search_text=LOWER(title || ' ' || first_user_message)
             WHERE id=?1 AND search_text=''",
            [id],
        )
        .map_err(|error| error.to_string())?;
        if let Some((provider, model)) = &update.model {
            tx.execute(
                "INSERT INTO session_models(session_id,provider,model,effort,service_tier) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(session_id) DO UPDATE SET provider=excluded.provider,model=excluded.model,effort=excluded.effort,service_tier=excluded.service_tier",
                params![id, provider, model, update.thinking_level, update.service_tier],
            ).map_err(|error| error.to_string())?;
        }
        if let Some(usage) = update.usage {
            tx.execute(
                "UPDATE sessions SET input_tokens=?2,output_tokens=?3,cache_read_tokens=?4,
                 cache_write_tokens=?5,total_tokens=?6,cost_micros=MAX(cost_micros,?7) WHERE id=?1",
                params![
                    id,
                    usage.input,
                    usage.output,
                    usage.cache_read,
                    usage.cache_write,
                    usage.total,
                    usage.cost_micros
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        // Keep the row protected from concurrent draft binding until read-back
        // completes: binding can merge this row into a different session ID.
        let mut session = Self::read_cached_sessions(&tx, Some(id))?
            .into_iter()
            .next()
            .ok_or("Updated session is missing")?;
        tx.commit().map_err(|error| error.to_string())?;
        session.is_running = update.is_running;
        Ok(session)
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
        let legacy_locators = legacy_session_locator_index(&transaction)?;
        for session in sessions {
            upsert_bound_session(&transaction, session, &legacy_locators)?;
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
        let mut missing = HashSet::new();
        for path in paths {
            let locator = crate::sessions::normalize_session_path(path);
            let queued = self
                .connection
                .query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM outbox o
                       JOIN sessions s ON s.id = o.session_id
                      WHERE s.locator=?1 AND o.state IN ('queued','sending')
                     )",
                    [locator.to_string_lossy()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|error| format!("check queued prompts for {}: {error}", path.display()))?;
            if queued {
                return Ok(true);
            }
            missing.insert(locator);
        }
        if missing.is_empty() {
            return Ok(false);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.locator FROM outbox o
                   JOIN sessions s ON s.id=o.session_id
                  WHERE o.state IN ('queued','sending') AND s.locator IS NOT NULL",
            )
            .map_err(|error| format!("prepare legacy queued locator index: {error}"))?;
        let locators = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query legacy queued locator index: {error}"))?;
        for locator in locators {
            let locator =
                locator.map_err(|error| format!("decode legacy queued locator: {error}"))?;
            if missing.contains(&crate::sessions::normalize_session_path(Path::new(
                &locator,
            ))) {
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
        let mut missing = Vec::new();
        for (source, target) in paths {
            let source_text = crate::sessions::normalize_session_path(source);
            let target_text = crate::sessions::normalize_session_path(target);
            let updated = transaction
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
            if updated == 0 {
                missing.push((source_text, target_text));
            }
        }
        if !missing.is_empty() {
            let legacy_locators = legacy_session_locator_index(&transaction)?;
            for (source, target) in missing {
                let Some(id) = legacy_session_id_from_index(&legacy_locators, &source, None)?
                else {
                    continue;
                };
                transaction
                    .execute(
                        "UPDATE sessions SET locator=?2, project_id=?3 WHERE id=?1",
                        params![id, target.to_string_lossy(), project_id],
                    )
                    .map_err(|error| {
                        format!(
                            "relocate legacy session state {} to {}: {error}",
                            source.display(),
                            target.display()
                        )
                    })?;
            }
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
        let mut missing = Vec::new();
        for path in paths {
            let locator = crate::sessions::normalize_session_path(path);
            let deleted = transaction
                .execute(
                    "DELETE FROM sessions WHERE locator=?1",
                    [locator.to_string_lossy()],
                )
                .map_err(|error| format!("delete saved state for {}: {error}", path.display()))?;
            if deleted == 0 {
                missing.push(locator);
            }
        }
        if !missing.is_empty() {
            let legacy_locators = legacy_session_locator_index(&transaction)?;
            for locator in missing {
                let Some(id) = legacy_session_id_from_index(&legacy_locators, &locator, None)?
                else {
                    continue;
                };
                transaction
                    .execute("DELETE FROM sessions WHERE id=?1", [id])
                    .map_err(|error| {
                        format!(
                            "delete legacy saved state for {}: {error}",
                            locator.display()
                        )
                    })?;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session state deletion: {error}"))
    }

    pub(crate) fn set_session_archived(&self, path: &Path, archived: bool) -> Result<(), String> {
        let locator = crate::sessions::normalize_session_path(path);
        let archived_at = archived.then_some(now_ms()).map(u64_to_i64);
        let updated = self
            .connection
            .execute(
                "UPDATE sessions SET archived_at=?2 WHERE locator=?1",
                params![locator.to_string_lossy(), archived_at],
            )
            .map_err(|error| format!("update archived state for {}: {error}", path.display()))?;
        if updated > 0 {
            return Ok(());
        }
        let legacy_locators = legacy_session_locator_index(&self.connection)?;
        let Some(id) = legacy_session_id_from_index(&legacy_locators, &locator, None)? else {
            return Ok(());
        };
        self.connection
            .execute(
                "UPDATE sessions SET locator=?2, archived_at=?3 WHERE id=?1",
                params![id, locator.to_string_lossy(), archived_at],
            )
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "update legacy archived state for {}: {error}",
                    path.display()
                )
            })
    }
}

fn upsert_bound_session(
    transaction: &Transaction<'_>,
    session: &SessionSummary,
    legacy_locators: &LegacyLocatorIndex,
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
    let existing = if existing.is_some() {
        existing
    } else {
        legacy_session_id_from_index(legacy_locators, &locator, Some(&session.harness))?
    };
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

type LegacyLocatorIndex = BTreeMap<PathBuf, BTreeMap<String, Vec<i64>>>;

fn legacy_session_locator_index(connection: &Connection) -> Result<LegacyLocatorIndex, String> {
    let mut statement = connection
        .prepare("SELECT id, harness, locator FROM sessions WHERE locator IS NOT NULL")
        .map_err(|error| format!("prepare legacy locator index: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("query legacy locator index: {error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("decode legacy locator index: {error}"))?;
    let mut index = BTreeMap::new();
    for (id, harness, locator) in rows {
        index
            .entry(crate::sessions::normalize_session_path(Path::new(&locator)))
            .or_insert_with(BTreeMap::new)
            .entry(harness)
            .or_insert_with(Vec::new)
            .push(id);
    }
    Ok(index)
}

fn legacy_session_id_from_index(
    index: &LegacyLocatorIndex,
    locator: &Path,
    harness: Option<&str>,
) -> Result<Option<i64>, String> {
    let Some(harnesses) = index.get(locator) else {
        return Ok(None);
    };
    match harness {
        Some(harness) => legacy_session_id(
            harnesses
                .get(harness)
                .into_iter()
                .flat_map(|ids| ids.iter().copied()),
            locator,
            Some(harness),
        ),
        None => legacy_session_id(
            harnesses.values().flat_map(|ids| ids.iter().copied()),
            locator,
            None,
        ),
    }
}

fn legacy_session_id(
    mut ids: impl Iterator<Item = i64>,
    locator: &Path,
    harness: Option<&str>,
) -> Result<Option<i64>, String> {
    let Some(id) = ids.next() else {
        return Ok(None);
    };
    if ids.next().is_none() {
        return Ok(Some(id));
    }
    Err(format!(
        "legacy session locator is ambiguous{}: {}",
        harness
            .map(|harness| format!(" for {harness}"))
            .unwrap_or_default(),
        locator.display()
    ))
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    let id = row.get::<_, i64>(0)?;
    let locator = row.get::<_, String>(1)?;
    let project = row.get::<_, String>(2)?;
    let provider = row.get::<_, Option<String>>(18)?;
    let model = row.get::<_, Option<String>>(19)?;
    let effort = row.get::<_, Option<String>>(20)?;
    let mut session = SessionSummary::from_cached_for_harness(
        row.get(21)?,
        row.get(17)?,
        crate::sessions::normalize_session_path(Path::new(&locator)),
        crate::sessions::normalize_session_path(Path::new(&project)),
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

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
