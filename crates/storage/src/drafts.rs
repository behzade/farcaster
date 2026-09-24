use super::*;

impl StateStore {
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
                        s.title, s.submitted, s.profile_id
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
            });
        }
        Ok(drafts)
    }
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
        "INSERT INTO sessions(id,project_id,harness,client_key,title,modified_ms,created_ms,submitted,profile_id)
         VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8)
         ON CONFLICT(id) DO UPDATE SET
           project_id=excluded.project_id, harness=excluded.harness, client_key=excluded.client_key,
           title=COALESCE(NULLIF(excluded.title,''),sessions.title), submitted=excluded.submitted,
           profile_id=excluded.profile_id",
        params![id,project_id,draft.harness.map(Backend::as_str).unwrap_or(""),draft.id,draft.title.as_deref().unwrap_or(""),
                u64_to_i64(draft.created_ms),draft.submitted,draft.profile_id],
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
