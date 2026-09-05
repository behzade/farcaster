use super::*;

impl StateStore {
    pub(crate) fn cached_sessions(&self, query: &str) -> Result<Vec<SessionSummary>, String> {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.cached_sessions");
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, path, project, title, first_user_message, timestamp,
                        parent_session, modified_ms, message_count, input_tokens,
                        output_tokens, cache_read_tokens, cache_write_tokens,
                        total_tokens, cost_micros, search_text, settled_ms IS NOT NULL,
                        is_running, app_session_id, harness
                   FROM sessions
                  ORDER BY modified_ms DESC, timestamp DESC",
            )
            .map_err(|error| format!("prepare cached sessions: {error}"))?;
        let rows = statement
            .query_map([], row_to_session)
            .map_err(|error| format!("query cached sessions: {error}"))?;
        let mut sessions = rows
            .map(|row| row.map_err(|error| format!("decode cached session: {error}")))
            .collect::<Result<Vec<_>, _>>()?;
        for link in self.load_worker_families()? {
            let Some(execution) = link.execution else {
                continue;
            };
            if let Some(session) = sessions.iter_mut().find(|session| {
                session.project == link.project
                    && session.harness == link.child_backend
                    && (session.id == link.child_session
                        || session.path == std::path::Path::new(&link.child_session))
            }) {
                session.model = Some((execution.provider, execution.model));
                session.thinking_level = execution.effort;
            }
        }
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
            .transaction()
            .map_err(|error| format!("start session index update: {error}"))?;
        let known = sessions
            .iter()
            .map(|session| session.path.to_string_lossy().into_owned())
            .collect::<HashSet<_>>();
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO sessions(
                       path, id, project, title, first_user_message, timestamp,
                       parent_session, modified_ms, file_size, message_count,
                       input_tokens, output_tokens, cache_read_tokens,
                       cache_write_tokens, total_tokens, cost_micros, search_text,
                       is_running, app_session_id, harness, settled_ms
                     ) VALUES(
                       ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                       ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
                     ) ON CONFLICT(path) DO UPDATE SET
                       id=excluded.id, project=excluded.project, title=excluded.title,
                       first_user_message=excluded.first_user_message,
                       timestamp=excluded.timestamp, parent_session=excluded.parent_session,
                       modified_ms=excluded.modified_ms, file_size=excluded.file_size,
                       message_count=excluded.message_count, input_tokens=excluded.input_tokens,
                       output_tokens=excluded.output_tokens,
                       cache_read_tokens=excluded.cache_read_tokens,
                       cache_write_tokens=excluded.cache_write_tokens,
                       total_tokens=excluded.total_tokens, cost_micros=excluded.cost_micros,
                       search_text=excluded.search_text, is_running=excluded.is_running,
                       app_session_id=excluded.app_session_id, harness=excluded.harness",
                )
                .map_err(|error| format!("prepare session index update: {error}"))?;
            for session in sessions {
                let (app_session_id, classify_import) =
                    ensure_session_app_session(&transaction, session).map_err(|error| {
                        format!("identify session {}: {error}", session.path.display())
                    })?;
                let settled_ms = (session.archived
                    || classify_import && imported_session_is_archived(session, SystemTime::now()))
                .then(now_ms);
                let size = std::fs::metadata(&session.path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                statement
                    .execute(params![
                        session.path.to_string_lossy(),
                        session.id,
                        session.project.to_string_lossy(),
                        session.title,
                        session.first_user_message,
                        session.timestamp,
                        session.parent_session,
                        system_time_ms(session.modified),
                        size,
                        usize_to_u64(session.message_count),
                        session.usage.input,
                        session.usage.output,
                        session.usage.cache_read,
                        session.usage.cache_write,
                        session.usage.total,
                        session.usage.cost_micros,
                        session.search_text(),
                        session.is_running,
                        app_session_id,
                        session.harness,
                        settled_ms,
                    ])
                    .map_err(|error| {
                        format!("index session {}: {error}", session.path.display())
                    })?;
                if classify_import {
                    transaction
                        .execute(
                            "UPDATE app_sessions SET import_classified=1 WHERE id=?1",
                            [app_session_id],
                        )
                        .map_err(|error| {
                            format!(
                                "classify imported session {}: {error}",
                                session.path.display()
                            )
                        })?;
                }
            }
        }
        if prune_missing {
            let mut paths = transaction
                .prepare("SELECT path FROM sessions")
                .map_err(|error| format!("read indexed paths: {error}"))?
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| format!("query indexed paths: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("decode indexed paths: {error}"))?;
            paths.retain(|path| !known.contains(path));
            for path in paths {
                transaction
                    .execute("DELETE FROM sessions WHERE path=?1", [path])
                    .map_err(|error| format!("remove stale session: {error}"))?;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session index: {error}"))
    }

    pub(crate) fn has_queued_prompts_for(&self, paths: &[PathBuf]) -> Result<bool, String> {
        for path in paths {
            let queued = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM outbox WHERE session_path=?1 AND state='queued')",
                    [path.to_string_lossy()],
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
        for (source, target) in paths {
            let source_text = source.to_string_lossy();
            let target_text = target.to_string_lossy();
            let source_target = format!("session:{source_text}");
            let target_target = format!("session:{target_text}");
            transaction
                .execute(
                    "UPDATE app_sessions SET session_path=?2 WHERE session_path=?1",
                    params![source_text, target_text],
                )
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE sessions SET path=?2, project=?3 WHERE path=?1",
                        params![source_text, target_text, target_project.to_string_lossy()],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE drafts SET session_path=?2, project=?3 WHERE session_path=?1",
                        params![source_text, target_text, target_project.to_string_lossy()],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE outbox SET target=?2, project=?3, session_path=?4 WHERE target=?1",
                        params![
                            source_target,
                            target_target,
                            target_project.to_string_lossy(),
                            target_text
                        ],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE prompt_presentations SET session_path=?2 WHERE session_path=?1",
                        params![source_text, target_text],
                    )
                })
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE composer_sessions SET target=?2 WHERE target=?1",
                        params![source_target, target_target],
                    )
                })
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
            let path_text = path.to_string_lossy();
            let target = format!("session:{path_text}");
            transaction
                .execute("DELETE FROM outbox WHERE session_path=?1", [&path_text])
                .and_then(|_| {
                    transaction.execute(
                        "DELETE FROM prompt_presentations WHERE session_path=?1",
                        [&path_text],
                    )
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM composer_sessions WHERE target=?1", [&target])
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM drafts WHERE session_path=?1", [&path_text])
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM sessions WHERE path=?1", [&path_text])
                })
                .and_then(|_| {
                    transaction.execute(
                        "DELETE FROM app_sessions WHERE session_path=?1",
                        [&path_text],
                    )
                })
                .map_err(|error| format!("delete saved state for {}: {error}", path.display()))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit session state deletion: {error}"))
    }

    pub(crate) fn set_session_archived(&self, path: &Path, archived: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE sessions SET settled_ms=?2 WHERE path=?1",
                params![
                    path.to_string_lossy(),
                    archived.then_some(now_ms()).map(u64_to_i64)
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("update archived state for {}: {error}", path.display()))
    }
}

fn ensure_session_app_session(
    transaction: &Transaction<'_>,
    session: &SessionSummary,
) -> rusqlite::Result<(i64, bool)> {
    let path = crate::sessions::normalize_session_path(&session.path);
    if let Some((id, classified)) = transaction
        .query_row(
            "SELECT id, import_classified FROM app_sessions WHERE session_path=?1",
            [path.to_string_lossy()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?
    {
        return Ok((id, !classified));
    }
    if session.app_session_id > 0 {
        transaction.execute(
            "INSERT OR IGNORE INTO app_sessions(id, session_path, created_ms, harness)
             VALUES(?1, ?2, ?3, ?4)",
            params![
                session.app_session_id,
                path.to_string_lossy(),
                u64_to_i64(system_time_ms(session.modified)),
                session.harness
            ],
        )?;
        transaction.execute(
            "UPDATE app_sessions SET session_path=?2 WHERE id=?1 AND session_path IS NULL",
            params![session.app_session_id, path.to_string_lossy()],
        )?;
    } else {
        transaction.execute(
            "INSERT OR IGNORE INTO app_sessions(session_path, created_ms, harness) VALUES(?1, ?2, ?3)",
            params![
                path.to_string_lossy(),
                u64_to_i64(system_time_ms(session.modified)),
                session.harness
            ],
        )?;
    }
    transaction
        .query_row(
            "SELECT id, import_classified FROM app_sessions WHERE session_path=?1",
            [path.to_string_lossy()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        )
        .map(|(id, classified)| (id, !classified))
}

fn imported_session_is_archived(session: &SessionSummary, now: SystemTime) -> bool {
    now.duration_since(session.modified).unwrap_or_default() > ACTIVE_IMPORT_WINDOW
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary::from_cached_for_harness(
        row.get(0)?,
        row.get(19)?,
        PathBuf::from(row.get::<_, String>(1)?),
        PathBuf::from(row.get::<_, String>(2)?),
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
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
        row.get(17)?,
        row.get(15)?,
    )
    .with_app_session_id(row.get(18)?))
}
