use super::*;

#[derive(Clone, Default)]
pub struct SessionStateChanges {
    pub projects: Option<projects::ProjectList>,
    pub drafts: BTreeMap<String, Option<DraftSession>>,
    pub folders: Option<sessions::SessionFolders>,
}

impl SessionStateChanges {
    pub fn is_empty(&self) -> bool {
        self.projects.is_none() && self.drafts.is_empty() && self.folders.is_none()
    }
}

impl StateStore {
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

fn update_draft(tx: &Transaction<'_>, draft: &DraftSession) -> Result<(), String> {
    let existing = tx
        .query_row(
            "SELECT submitted, locator FROM sessions WHERE id=?1 AND client_key=?2",
            params![draft.app_session_id, draft.id],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|error| format!("read draft before saving: {error}"))?;
    let Some((submitted, locator)) = existing else {
        // Allocation is synchronous; a queued save must never recreate a deleted
        // draft or reattach a draft key removed by promotion.
        return Ok(());
    };
    if !submitted && locator.is_none() {
        save_draft(tx, draft)?;
    } else {
        // Runtime promotion can finish before this queued UI snapshot. Preserve
        // the canonical identity, settings and title instead of rolling them back.
        tx.execute(
            "UPDATE sessions SET submitted=MAX(submitted,?3),
                title=CASE WHEN title='' THEN COALESCE(?4,'') ELSE title END,
                archived_at=CASE WHEN locator IS NULL AND ?3 THEN ?5 ELSE archived_at END
             WHERE id=?1 AND client_key=?2",
            params![
                draft.app_session_id,
                draft.id,
                draft.submitted,
                draft.title,
                draft.archived.then_some(u64_to_i64(draft.created_ms))
            ],
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
