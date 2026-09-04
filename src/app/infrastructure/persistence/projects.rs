use super::*;

impl StateStore {
    pub(crate) fn allocate_app_session_id(
        &mut self,
        draft_id: &str,
        created_ms: u64,
    ) -> Result<i64, String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start application session allocation: {error}"))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO app_sessions(draft_id, created_ms) VALUES(?1, ?2)",
                params![draft_id, u64_to_i64(created_ms)],
            )
            .map_err(|error| format!("allocate application session for {draft_id}: {error}"))?;
        let id = transaction
            .query_row(
                "SELECT id FROM app_sessions WHERE draft_id=?1",
                [draft_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("read application session for {draft_id}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit application session allocation: {error}"))?;
        Ok(id)
    }

    pub(crate) fn load_registry(&self) -> Result<Registry, String> {
        let mut projects = Vec::new();
        let mut statement = self
            .connection
            .prepare("SELECT path FROM projects ORDER BY added_ms, path")
            .map_err(|error| format!("read projects: {error}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query projects: {error}"))?;
        for row in rows {
            if let Some(path) = existing_directory(&row.map_err(|error| error.to_string())?) {
                projects.push(path);
            }
        }
        let excluded_projects = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='excluded_projects'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load excluded projects: {error}"))?
            .map_or_else(
                || Ok(Vec::new()),
                |value| {
                    serde_json::from_str(&value)
                        .map_err(|error| format!("decode excluded projects: {error}"))
                },
            )?;
        let mut drafts = Vec::new();
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, app_session_id, harness, project, created_ms, submitted, session_path,
                        provisional_title
                   FROM drafts ORDER BY created_ms DESC",
            )
            .map_err(|error| format!("read drafts: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|error| format!("query drafts: {error}"))?;
        for row in rows {
            let (id, app_session_id, harness, project, created_ms, submitted, session_path, title) =
                row.map_err(|error| error.to_string())?;
            if let Some(project) = existing_directory(&project) {
                drafts.push(DraftSession {
                    id,
                    app_session_id,
                    harness,
                    project,
                    created_ms,
                    submitted,
                    session_path: session_path
                        .map(PathBuf::from)
                        .map(|path| crate::sessions::normalize_session_path(&path)),
                    title,
                });
            }
        }
        Ok(Registry {
            projects,
            excluded_projects,
            drafts,
        })
    }

    pub(crate) fn save_registry(&mut self, registry: &Registry) -> Result<(), String> {
        let excluded_projects = serde_json::to_string(&registry.excluded_projects)
            .map_err(|error| format!("encode excluded projects: {error}"))?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("start registry update: {error}"))?;
        transaction
            .execute("DELETE FROM projects", [])
            .map_err(|error| format!("clear projects: {error}"))?;
        transaction
            .execute("DELETE FROM drafts", [])
            .map_err(|error| format!("clear drafts: {error}"))?;
        transaction
            .execute(
                "INSERT INTO meta(key, value) VALUES('excluded_projects', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [excluded_projects],
            )
            .map_err(|error| format!("save excluded projects: {error}"))?;
        let now = now_ms();
        for (index, project) in registry.projects.iter().enumerate() {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO projects(path, added_ms) VALUES(?1, ?2)",
                    params![project.to_string_lossy(), now.saturating_add(index as u64)],
                )
                .map_err(|error| format!("save project {}: {error}", project.display()))?;
        }
        for draft in &registry.drafts {
            let session_path = draft
                .session_path
                .as_ref()
                .map(|path| crate::sessions::normalize_session_path(path));
            let app_session_id =
                ensure_draft_app_session(&transaction, draft, session_path.as_deref())
                    .map_err(|error| format!("identify draft {}: {error}", draft.id))?;
            transaction
                .execute(
                    "INSERT INTO drafts(
                       id, app_session_id, harness, project, created_ms, submitted, session_path,
                       provisional_title
                     ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        draft.id,
                        app_session_id,
                        draft.harness,
                        draft.project.to_string_lossy(),
                        draft.created_ms,
                        draft.submitted,
                        session_path.as_ref().map(|path| path.to_string_lossy()),
                        draft.title
                    ],
                )
                .map_err(|error| format!("save draft {}: {error}", draft.id))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit registry update: {error}"))
    }
}

fn ensure_draft_app_session(
    transaction: &Transaction<'_>,
    draft: &DraftSession,
    session_path: Option<&Path>,
) -> rusqlite::Result<i64> {
    if draft.app_session_id > 0 {
        transaction.execute(
            "INSERT OR IGNORE INTO app_sessions(id, draft_id, created_ms, harness) VALUES(?1, ?2, ?3, ?4)",
            params![
                draft.app_session_id,
                draft.id,
                u64_to_i64(draft.created_ms),
                draft.harness
            ],
        )?;
    }
    transaction.execute(
        "INSERT OR IGNORE INTO app_sessions(draft_id, created_ms, harness) VALUES(?1, ?2, ?3)",
        params![draft.id, u64_to_i64(draft.created_ms), draft.harness],
    )?;
    let app_session_id = transaction.query_row(
        "SELECT id FROM app_sessions WHERE draft_id=?1",
        [&draft.id],
        |row| row.get::<_, i64>(0),
    )?;
    if let Some(path) = session_path {
        associate_app_session(transaction, app_session_id, &draft.id, path)?;
    }
    Ok(app_session_id)
}

pub(super) fn associate_app_session(
    transaction: &Transaction<'_>,
    app_session_id: i64,
    draft_id: &str,
    session_path: &Path,
) -> rusqlite::Result<()> {
    let path = crate::sessions::normalize_session_path(session_path);
    let existing = transaction
        .query_row(
            "SELECT id FROM app_sessions WHERE session_path=?1",
            [path.to_string_lossy()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(existing) = existing.filter(|existing| *existing != app_session_id) {
        transaction.execute(
            "UPDATE sessions SET app_session_id=?2 WHERE app_session_id=?1 OR path=?3",
            params![existing, app_session_id, path.to_string_lossy()],
        )?;
        transaction.execute("DELETE FROM app_sessions WHERE id=?1", [existing])?;
    }
    transaction.execute(
        "UPDATE app_sessions
            SET draft_id=COALESCE(draft_id, ?2), session_path=?3, import_classified=1
          WHERE id=?1",
        params![app_session_id, draft_id, path.to_string_lossy()],
    )?;
    transaction.execute(
        "UPDATE sessions SET app_session_id=?2 WHERE path=?1",
        params![path.to_string_lossy(), app_session_id],
    )?;
    Ok(())
}

fn existing_directory(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path).canonicalize().ok()?;
    path.is_dir().then_some(path)
}
