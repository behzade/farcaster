use super::*;

impl StateStore {
    /// Session provisioning is a caller-binding lifecycle operation, not an
    /// effect of a tool submission. Use the same scoped identity as families.
    pub fn register_caller_session(
        &self,
        caller: &crate::agents::CallerContext,
    ) -> Result<i64, String> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(|e| format!("register caller session: {e}"))?;
        let id = register_caller_session_in(&tx, &self.image_directory, caller)?;
        tx.commit()
            .map_err(|e| format!("commit caller session: {e}"))?;
        Ok(id)
    }

    pub fn register_execution_for_caller(
        &self,
        caller: &crate::agents::CallerContext,
        execution: &crate::agents::ExecutionBinding,
    ) -> Result<i64, String> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(|e| format!("register execution turn: {e}"))?;
        let session_id = register_caller_session_in(&tx, &self.image_directory, caller)?;
        tx.execute(
            "INSERT INTO session_turns(id,session_id,prompt_id,started_ms) VALUES(?1,?2,?3,?4)",
            params![execution.turn_id, session_id, execution.prompt_id, now_ms()],
        )
        .map_err(|e| format!("register execution turn: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit execution turn: {e}"))?;
        Ok(session_id)
    }

    pub fn save_review(
        &self,
        caller: &crate::agents::CallerContext,
        execution: &crate::agents::ExecutionBinding,
        artifact: &serde_json::Value,
    ) -> Result<(), String> {
        let id = artifact
            .pointer("/farcaster_review/id")
            .and_then(serde_json::Value::as_str)
            .ok_or("review is missing its identity")?;
        // This lookup is deliberately read-only: no locator changes, provisioning,
        // or identity merges can happen as a side effect of submit_review.
        let changed = self
            .connection
            .execute(
                "INSERT INTO session_reviews(id,session_id,turn_id,artifact,created_ms)
            SELECT ?1,s.id,t.id,?3,?4 FROM sessions s JOIN projects p ON p.id=s.project_id
              JOIN session_turns t ON t.session_id=s.id AND t.id=?2
            WHERE s.harness=?5 AND p.path=?6
              AND ((?7 IS NOT NULL AND s.locator=?7)
                OR (?7 IS NULL AND (s.backend_id=?8 OR s.locator=?9)))",
                params![
                    id,
                    execution.turn_id,
                    artifact.to_string(),
                    now_ms(),
                    caller.backend.as_str(),
                    crate::sessions::normalize_session_path(&caller.project).to_string_lossy(),
                    caller.session_locator.as_ref().map(|path| {
                        crate::sessions::normalize_session_path(path)
                            .to_string_lossy()
                            .into_owned()
                    }),
                    caller.session,
                    crate::sessions::normalize_session_path(Path::new(&caller.session))
                        .to_string_lossy()
                ],
            )
            .map_err(|e| format!("save review submission: {e}"))?;
        if changed != 1 {
            return Err("review execution no longer belongs to the registered session".into());
        }
        Ok(())
    }
}

fn register_caller_session_in(
    tx: &Transaction<'_>,
    image_directory: &Path,
    caller: &crate::agents::CallerContext,
) -> Result<i64, String> {
    if let Some(locator) = &caller.session_locator
        && crate::agents::external_session_identity(locator)
            != Some((caller.backend, caller.session.clone()))
    {
        return Err("caller session locator does not match its native identity".into());
    }
    let project = ensure_project(tx, &caller.project, u64_to_i64(now_ms()))?;
    let root = image_directory
        .parent()
        .ok_or("session database has no parent")?
        .join("session-locators");
    if caller.session_locator.is_none() && !Path::new(&caller.session).is_absolute() {
        let encoded =
            url::form_urlencoded::byte_serialize(caller.session.as_bytes()).collect::<String>();
        let legacy = crate::sessions::normalize_session_path(
            &root.join(caller.backend.as_str()).join(encoded),
        );
        tx.execute("UPDATE sessions SET backend_id=?1 WHERE harness=?2 AND project_id=?3 AND locator=?4 AND backend_id IS NULL",
            params![caller.session,caller.backend.as_str(),project,legacy.to_string_lossy()]).map_err(|e| format!("bind native session identity: {e}"))?;
    }
    let root = super::identity::family_locator_root(
        &crate::sessions::normalize_session_path(&root),
        &caller.project,
    );
    let identity = caller
        .session_locator
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| caller.session.clone());
    ensure_locator_session(tx, caller.backend, &identity, project, &root)
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
