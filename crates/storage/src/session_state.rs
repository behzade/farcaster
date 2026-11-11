use super::*;

#[derive(Clone, Default)]
pub struct SessionStateChanges {
    pub projects: Option<projects::ProjectList>,
    pub drafts: BTreeMap<String, Option<DraftSession>>,
    pub folders: Option<sessions::SessionFolders>,
    pub selected_sessions: BTreeMap<PathBuf, Option<i64>>,
    pub last_selected_project: Option<PathBuf>,
}

impl SessionStateChanges {
    pub fn is_empty(&self) -> bool {
        self.projects.is_none()
            && self.drafts.is_empty()
            && self.folders.is_none()
            && self.selected_sessions.is_empty()
            && self.last_selected_project.is_none()
    }
}

impl StateStore {
    pub fn load_last_selected_project(&self) -> Result<Option<PathBuf>, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key='last_selected_project'",
                [],
                |row| row.get::<_, String>(0).map(PathBuf::from),
            )
            .optional()
            .map_err(|error| format!("load last selected project: {error}"))
    }

    pub fn load_selected_session(&self, project: &Path) -> Result<Option<i64>, String> {
        let project = sessions::normalize_session_path(project);
        self.connection
            .query_row(
                "SELECT s.id FROM meta m
                 JOIN sessions s ON s.id=CAST(m.value AS INTEGER)
                 JOIN projects p ON p.id=s.project_id
                 WHERE m.key=?1 AND p.path=?2 AND s.archived_at IS NULL",
                params![selected_session_key(&project), project.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("load selected session: {error}"))
    }

    pub fn set_app_session_archived(
        &mut self,
        app_session_id: sessions::AppSessionId,
        archived: bool,
    ) -> Result<(), String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session archive intent: {error}"))?;
        apply_archive_intent(&tx, app_session_id.get(), archived)?;
        tx.commit()
            .map_err(|error| format!("commit session archive intent: {error}"))
    }

    pub fn save_session_changes(&mut self, changes: &SessionStateChanges) -> Result<(), String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session state save: {error}"))?;
        if let Some(projects) = &changes.projects {
            project_storage::save_projects(&tx, &projects.projects, &projects.excluded_projects)?;
        }
        for (key, draft) in &changes.drafts {
            match draft {
                Some(draft) => update_draft(&tx, draft)?,
                None => remove_draft_row(&tx, key)?,
            }
        }
        for (project, selected) in &changes.selected_sessions {
            let key = selected_session_key(project);
            if let Some(id) = selected {
                tx.execute(
                    "INSERT INTO meta(key,value) VALUES(?1,?2)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![key, id.to_string()],
                )
            } else {
                tx.execute("DELETE FROM meta WHERE key=?1", [key])
            }
            .map_err(|error| format!("save selected session: {error}"))?;
        }
        if let Some(project) = &changes.last_selected_project {
            tx.execute(
                "INSERT INTO meta(key,value) VALUES('last_selected_project',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [sessions::normalize_session_path(project)
                    .to_string_lossy()
                    .as_ref()],
            )
            .map_err(|error| format!("save last selected project: {error}"))?;
        }
        if let Some(folders) = &changes.folders {
            let json = serde_json::to_string(folders)
                .map_err(|error| format!("encode session folders: {error}"))?;
            tx.execute(
                "INSERT INTO meta(key,value) VALUES('session_folders',?1)
                ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(|error| format!("save session folders: {error}"))?;
        }
        tx.commit()
            .map_err(|error| format!("commit session state save: {error}"))
    }
}

fn selected_session_key(project: &Path) -> String {
    format!(
        "selected_session:{}",
        sessions::normalize_session_path(project).display()
    )
}

pub(super) fn apply_archive_intent(
    tx: &Transaction<'_>,
    session_id: i64,
    archived: bool,
) -> Result<(), String> {
    let now = u64_to_i64(now_ms());
    let updated = tx
        .execute(
            "UPDATE sessions SET archived_at=?2 WHERE id=?1",
            params![session_id, archived.then_some(now)],
        )
        .map_err(|error| format!("apply session archive intent: {error}"))?;
    if updated == 0 {
        return Err("The chat is no longer available to archive".into());
    }
    tx.execute(
        "INSERT INTO session_events(session_id,seq,t,schema_version,body)
         SELECT ?1, COALESCE(MAX(seq),0)+1, ?2, 1,
                json_object('type','session_archive_intent','archived',json(?3))
           FROM session_events WHERE session_id=?1",
        params![session_id, now, if archived { "true" } else { "false" }],
    )
    .map_err(|error| format!("record session archive intent: {error}"))?;
    Ok(())
}

fn update_draft(tx: &Transaction<'_>, draft: &DraftSession) -> Result<(), String> {
    let existing = tx
        .query_row(
            "SELECT submitted, locator, archived_at IS NOT NULL FROM sessions WHERE id=?1 AND client_key=?2",
            params![draft.app_session_id, draft.id],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, bool>(2)?)),
        )
        .optional()
        .map_err(|error| format!("read draft before saving: {error}"))?;
    let Some((submitted, locator, archived)) = existing else {
        return Ok(());
    };
    if !submitted && locator.is_none() {
        let mut snapshot = draft.clone();
        snapshot.archived = archived;
        save_draft(tx, &snapshot)?;
    } else {
        tx.execute(
            "UPDATE sessions SET submitted=MAX(submitted,?3),
                title=CASE WHEN title='' THEN COALESCE(?4,'') ELSE title END
             WHERE id=?1 AND client_key=?2",
            params![draft.app_session_id, draft.id, draft.submitted, draft.title,],
        )
        .map_err(|error| format!("save submitted draft state: {error}"))?;
        if locator.is_none()
            && let Some(path) = &draft.session_path
        {
            bind_locator(tx, &draft.id, &sessions::normalize_session_path(path))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_state_tests.rs"]
mod tests;
