use super::*;

impl StateStore {
    pub(crate) fn allocate_app_session_id(&mut self, draft: &DraftSession) -> Result<i64, String> {
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

    pub(crate) fn load_registry(&self) -> Result<Registry, String> {
        let mut projects = Vec::new();
        let mut excluded_projects = Vec::new();
        let mut statement = self
            .connection
            .prepare("SELECT path, deleted_at FROM projects ORDER BY added_ms, path")
            .map_err(|error| format!("read projects: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
            })
            .map_err(|error| format!("query projects: {error}"))?;
        for row in rows {
            let (path, deleted_at) = row.map_err(|error| error.to_string())?;
            let Some(path) = existing_directory(&path) else {
                continue;
            };
            if deleted_at.is_some() {
                excluded_projects.push(path);
            } else {
                projects.push(path);
            }
        }
        let mut drafts = Vec::new();
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.id, s.client_key, s.harness, p.path, s.created_ms, s.locator,
                        s.title, s.submitted
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
                ))
            })
            .map_err(|error| format!("query drafts: {error}"))?;
        for row in rows {
            let (id, client_key, harness, project, created_ms, locator, title, submitted) =
                row.map_err(|error| error.to_string())?;
            if let Some(project) = existing_directory(&project) {
                drafts.push(DraftSession {
                    id: client_key,
                    app_session_id: id,
                    harness,
                    project,
                    created_ms,
                    submitted,
                    session_path: locator
                        .map(PathBuf::from)
                        .map(|path| crate::sessions::normalize_session_path(&path)),
                    title: (!title.is_empty()).then_some(title),
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start registry update: {error}"))?;
        let now = u64_to_i64(now_ms());
        transaction
            .execute(
                "UPDATE projects SET deleted_at=COALESCE(deleted_at, ?1)",
                [now],
            )
            .map_err(|error| format!("hide projects: {error}"))?;
        for (index, project) in registry.projects.iter().enumerate() {
            let project_id =
                ensure_project(&transaction, project, now.saturating_add(index as i64))?;
            transaction
                .execute(
                    "UPDATE projects SET deleted_at=NULL WHERE id=?1",
                    [project_id],
                )
                .map_err(|error| format!("restore registered project: {error}"))?;
        }
        for project in &registry.excluded_projects {
            transaction
                .execute(
                    "INSERT INTO projects(path, added_ms, deleted_at) VALUES(?1, ?2, ?2)
                     ON CONFLICT(path) DO UPDATE SET deleted_at=excluded.deleted_at",
                    params![project.to_string_lossy(), now],
                )
                .map_err(|error| format!("exclude project {}: {error}", project.display()))?;
        }
        let kept = registry
            .drafts
            .iter()
            .map(|draft| draft.id.as_str())
            .collect::<HashSet<_>>();
        let stale = transaction
            .prepare("SELECT client_key FROM sessions WHERE client_key IS NOT NULL")
            .map_err(|error| format!("read draft keys: {error}"))?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query draft keys: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("decode draft keys: {error}"))?;
        for key in stale {
            if !kept.contains(key.as_str()) {
                transaction
                    .execute(
                        "UPDATE sessions SET client_key=NULL WHERE client_key=?1 AND locator IS NOT NULL",
                        [&key],
                    )
                    .map_err(|error| format!("detach draft {key}: {error}"))?;
                transaction
                    .execute(
                        "DELETE FROM sessions WHERE client_key=?1 AND locator IS NULL",
                        [&key],
                    )
                    .map_err(|error| format!("drop draft {key}: {error}"))?;
            }
        }
        for draft in &registry.drafts {
            save_draft(&transaction, draft)?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit registry update: {error}"))
    }
}

fn existing_directory(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path).canonicalize().ok()?;
    path.is_dir().then_some(path)
}

fn save_draft(tx: &Transaction<'_>, draft: &DraftSession) -> Result<i64, String> {
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
        "INSERT INTO sessions(id,project_id,harness,client_key,title,modified_ms,created_ms,submitted)
         VALUES(?1,?2,?3,?4,?5,?6,?6,?7)
         ON CONFLICT(id) DO UPDATE SET
           project_id=excluded.project_id, harness=excluded.harness, client_key=excluded.client_key,
           title=COALESCE(NULLIF(excluded.title,''),sessions.title), submitted=excluded.submitted",
        params![id,project_id,draft.harness,draft.id,draft.title.as_deref().unwrap_or(""),
                u64_to_i64(draft.created_ms),draft.submitted],
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
