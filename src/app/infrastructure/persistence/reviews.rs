use super::*;
use crate::app::reviews::delivery::Submission;

const SESSION_REVIEWS_SQL: &str = "SELECT r.id,r.artifact,r.turn_id,t.prompt_id,t.user_ordinal
    FROM session_reviews r LEFT JOIN session_turns t ON t.id=r.turn_id
    WHERE r.session_id IN (
        SELECT s.id FROM sessions s JOIN projects p ON p.id=s.project_id
         WHERE s.harness=?1 AND p.path=?2 AND s.locator=?3
        UNION
        SELECT s.id FROM sessions s JOIN projects p ON p.id=s.project_id
         WHERE s.harness=?1 AND p.path=?2 AND s.backend_id=?4
    ) ORDER BY r.seq";

impl StateStore {
    /// Session provisioning is a caller-binding lifecycle operation, not an
    /// effect of a tool submission. Use the same scoped identity as families.
    pub(crate) fn register_caller_session(
        &self,
        caller: &crate::agents::CallerContext,
    ) -> Result<i64, String> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(|e| format!("register caller session: {e}"))?;
        let project = ensure_project(&tx, &caller.project, u64_to_i64(now_ms()))?;
        let root = self
            .image_directory
            .parent()
            .ok_or("session database has no parent")?
            .join("session-locators");
        if !Path::new(&caller.session).is_absolute() {
            let encoded =
                url::form_urlencoded::byte_serialize(caller.session.as_bytes()).collect::<String>();
            let legacy = crate::sessions::normalize_session_path(
                &root.join(caller.backend.as_str()).join(encoded),
            );
            tx.execute("UPDATE sessions SET backend_id=?1 WHERE harness=?2 AND project_id=?3 AND locator=?4 AND backend_id IS NULL",
                params![caller.session,caller.backend,project,legacy.to_string_lossy()]).map_err(|e| format!("bind native session identity: {e}"))?;
        }
        let root = super::identity::family_locator_root(
            &crate::sessions::normalize_session_path(&root),
            &caller.project,
        );
        let id = ensure_locator_session(&tx, caller.backend, &caller.session, project, &root)?;
        tx.commit()
            .map_err(|e| format!("commit caller session: {e}"))?;
        Ok(id)
    }

    pub(crate) fn register_execution(
        &self,
        execution: &crate::agents::ExecutionBinding,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO session_turns(id,session_id,prompt_id,started_ms) VALUES(?1,?2,?3,?4)",
                params![
                    execution.turn_id,
                    execution.session_record,
                    execution.prompt_id,
                    now_ms()
                ],
            )
            .map(|_| ())
            .map_err(|e| format!("register execution turn: {e}"))
    }

    pub(crate) fn save_review(
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
            WHERE s.harness=?5 AND p.path=?6 AND (s.backend_id=?7 OR s.locator=?8)",
                params![
                    id,
                    execution.turn_id,
                    artifact.to_string(),
                    now_ms(),
                    caller.backend,
                    crate::sessions::normalize_session_path(&caller.project).to_string_lossy(),
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

    pub(crate) fn record_review_position(
        &self,
        turn_id: &str,
        ordinal: usize,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE session_turns SET user_ordinal=?2 WHERE id=?1 AND user_ordinal IS NULL",
                params![turn_id, usize_to_i64(ordinal)],
            )
            .map(|_| ())
            .map_err(|e| format!("save review turn position: {e}"))
    }

    pub(crate) fn session_reviews(
        &self,
        backend: Backend,
        project: &Path,
        session: &Path,
    ) -> Result<Vec<Submission>, String> {
        let native_id = crate::agents::external_session_identity(session)
            .filter(|(harness, _)| *harness == backend)
            .map(|(_, id)| id);
        let mut statement = self
            .connection
            .prepare(SESSION_REVIEWS_SQL)
            .map_err(|e| format!("prepare session reviews: {e}"))?;
        let rows = statement
            .query_map(
                params![
                    backend,
                    crate::sessions::normalize_session_path(project).to_string_lossy(),
                    crate::sessions::normalize_session_path(session).to_string_lossy(),
                    native_id
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<usize>>(4)?,
                    ))
                },
            )
            .map_err(|e| format!("load session reviews: {e}"))?;
        rows.map(|row| {
            let (id, artifact, turn_id, prompt_id, user_ordinal) =
                row.map_err(|e| e.to_string())?;
            Ok(Submission {
                id,
                artifact: serde_json::from_str(&artifact)
                    .map_err(|e| format!("decode stored review: {e}"))?,
                turn_id,
                prompt_id,
                user_ordinal,
            })
        })
        .collect()
    }
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
