use super::*;

impl StateStore {
    #[cfg(test)]
    pub(super) fn draft_session_target(
        &self,
        key: &str,
    ) -> Result<Option<sessions::SessionTarget>, String> {
        let row = self
            .connection
            .query_row(
                "SELECT harness, locator, COALESCE(backend_id, locator) FROM sessions
                  WHERE client_key=?1 AND locator IS NOT NULL",
                [key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("load draft session target: {error}"))?;
        row.map(|(harness, path, id)| {
            Ok(sessions::SessionTarget {
                harness: harness.parse()?,
                path: sessions::normalize_session_path(Path::new(&path)),
                id,
            })
        })
        .transpose()
    }

    pub fn draft_profile_id(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT profile_id FROM sessions WHERE client_key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(|error| format!("load draft harness profile: {error}"))
    }

    pub fn allocate_app_session_id(&mut self, draft: &DraftSession) -> Result<i64, String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session allocation: {error}"))?;
        let id = save_draft(&transaction, draft)?;
        transaction
            .commit()
            .map_err(|error| format!("commit session allocation: {error}"))?;
        Ok(id)
    }

    pub fn remove_draft(&mut self, key: &str) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start draft removal: {error}"))?;
        remove_draft_row(&transaction, key)?;
        transaction
            .commit()
            .map_err(|error| format!("commit draft removal: {error}"))
    }

    pub fn load_drafts(&self) -> Result<Vec<DraftSession>, String> {
        let mut drafts = Vec::new();
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.id, s.client_key, s.harness, p.path, s.created_ms, s.locator,
                        s.title, s.submitted, s.profile_id, s.archived_at IS NOT NULL
                   FROM sessions s
                   JOIN projects p ON p.id = s.project_id
                  WHERE s.client_key IS NOT NULL
                  ORDER BY s.created_ms DESC",
            )
            .map_err(|error| format!("read drafts: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, bool>(9)?,
                ))
            })
            .map_err(|error| format!("query drafts: {error}"))?;
        for row in rows {
            let (
                id,
                client_key,
                harness,
                project,
                created_ms,
                locator,
                title,
                submitted,
                profile_id,
                archived,
            ) = row.map_err(|error| error.to_string())?;
            drafts.push(DraftSession {
                id: client_key,
                app_session_id: id,
                harness: if harness.is_empty() {
                    None
                } else {
                    Some(harness.parse()?)
                },
                profile_id,
                project: crate::sessions::normalize_session_path(Path::new(&project)),
                created_ms,
                submitted,
                session_path: locator
                    .map(PathBuf::from)
                    .map(|path| crate::sessions::normalize_session_path(&path)),
                title: (!title.is_empty()).then_some(title),
                archived,
            });
        }
        Ok(drafts)
    }
}

pub(super) fn register_prompt_caller(
    tx: &Transaction<'_>,
    draft: &str,
    caller: &agents::CallerContext,
) -> Result<i64, String> {
    let path = caller
        .session_locator
        .as_deref()
        .unwrap_or(Path::new(&caller.session));
    if caller.parent_worker_id.is_some() || !path.is_absolute() {
        return Err("a prompt draft requires a main session with an exact locator".into());
    }
    let id = bind_prompt_locator(
        tx,
        draft,
        caller.backend,
        &caller.project,
        caller.harness_profile_id.as_deref(),
        path,
    )?;
    if caller.app_session_id.is_some_and(|hint| hint.get() != id) {
        return Err("caller application identity conflicts with its draft".into());
    }
    if caller.session_locator.is_some() {
        let changed = tx
            .execute(
                "UPDATE sessions SET backend_id=?2
                  WHERE id=?1 AND (backend_id IS NULL OR backend_id=?2)",
                params![id, caller.session],
            )
            .map_err(|error| format!("save draft native ID: {error}"))?;
        if changed == 0 {
            return Err("caller native ID conflicts with its draft".into());
        }
    }
    Ok(id)
}

pub(super) fn bind_prompt_locator(
    tx: &Transaction<'_>,
    draft: &str,
    harness: Backend,
    project: &Path,
    profile_id: Option<&str>,
    path: &Path,
) -> Result<i64, String> {
    let path = sessions::normalize_session_path(path);
    let project = sessions::normalize_session_path(project);
    if sessions::profile_id_from_locator(&path)
        .as_deref()
        .is_some_and(|profile| Some(profile) != profile_id)
    {
        return Err("draft profile conflicts with the session locator".into());
    }
    // A catalog row may already have this locator. Merge it only within the
    // draft's project and profile, and only if another draft does not own it.
    let (id, existing): (i64, Option<i64>) = tx.query_row(
        "SELECT d.id, s.id FROM sessions d JOIN projects p ON p.id=d.project_id
           LEFT JOIN sessions s ON s.harness=d.harness AND s.locator=?5 AND s.id<>d.id
          WHERE d.client_key=?1 AND d.harness=?2 AND p.path=?3 AND d.profile_id IS ?4
            AND (d.locator IS NULL OR d.locator=?5)
            AND (s.id IS NULL OR (s.client_key IS NULL AND s.project_id=d.project_id AND s.profile_id IS ?4))",
        params![draft, harness.as_str(), project.to_string_lossy(), profile_id, path.to_string_lossy()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| format!("check draft locator: {error}"))?
        .ok_or_else(|| format!("Draft {draft} cannot bind to {}", path.display()))?;
    super::identity::attach_draft_locator(tx, id, existing, profile_id, &path)?;
    Ok(id)
}

pub(super) fn remove_draft_row(tx: &Transaction<'_>, key: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE sessions SET client_key=NULL WHERE client_key=?1 AND locator IS NOT NULL",
        [key],
    )
    .map_err(|error| format!("detach draft {key}: {error}"))?;
    tx.execute(
        "DELETE FROM sessions WHERE client_key=?1 AND locator IS NULL",
        [key],
    )
    .map_err(|error| format!("remove draft {key}: {error}"))?;
    Ok(())
}

pub(super) fn save_draft(tx: &Transaction<'_>, draft: &DraftSession) -> Result<i64, String> {
    let project_id = ensure_project(tx, &draft.project, u64_to_i64(draft.created_ms))?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM sessions WHERE client_key=?1",
            [&draft.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("find draft {}: {error}", draft.id))?;
    let id = existing.or((draft.app_session_id > 0).then_some(draft.app_session_id));
    tx.execute(
        "INSERT INTO sessions(id,project_id,harness,client_key,title,modified_ms,created_ms,submitted,profile_id,archived_at)
         VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8,?9)
         ON CONFLICT(id) DO UPDATE SET
           project_id=excluded.project_id, harness=excluded.harness, client_key=excluded.client_key,
           title=COALESCE(NULLIF(excluded.title,''),sessions.title), submitted=excluded.submitted,
           profile_id=excluded.profile_id,
           archived_at=CASE WHEN sessions.locator IS NULL THEN excluded.archived_at ELSE sessions.archived_at END",
        params![id,project_id,draft.harness.map(Backend::as_str).unwrap_or(""),draft.id,draft.title.as_deref().unwrap_or(""),
                u64_to_i64(draft.created_ms),draft.submitted,draft.profile_id,
                draft.archived.then_some(u64_to_i64(draft.created_ms))],
    ).map_err(|error| format!("save draft {}: {error}", draft.id))?;
    let id = id.unwrap_or_else(|| tx.last_insert_rowid());
    if let Some(locator) = &draft.session_path {
        bind_locator(
            tx,
            &draft.id,
            &crate::sessions::normalize_session_path(locator),
        )?;
    }
    Ok(id)
}
