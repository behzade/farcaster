use super::*;

/// Queued UI snapshots. Archive methods record explicit intent separately.
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
    /// Apply explicit archive intent to the durable chat, whether binding has
    /// happened yet or its draft key has already been removed by promotion.
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
    // A NULL archive value alone cannot distinguish an explicit unarchive
    // from an old snapshot. Merges and snapshots use this marker to preserve
    // the explicit decision.
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
        // Allocation is synchronous; a queued save must never recreate a deleted
        // draft or reattach a draft key removed by promotion.
        return Ok(());
    };
    if !submitted && locator.is_none() {
        let mut snapshot = draft.clone();
        snapshot.archived = archived;
        save_draft(tx, &snapshot)?;
    } else {
        // Runtime promotion can finish before this queued UI snapshot. Preserve
        // the canonical identity, settings and title instead of rolling them back.
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
