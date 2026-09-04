use super::*;

impl StateStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_prompt(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        self.enqueue_prompt_with_presentation(
            target, harness, project, session, mode, message, None, None, images,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_prompt_with_presentation(
        &self,
        target: &str,
        harness: &str,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        display_message: Option<&str>,
        invocation: Option<&str>,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        let images_json = serde_json::to_string(images)
            .map_err(|error| format!("encode prompt images: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO outbox(
                   target, harness, project, session_path, mode, message, display_message,
                   invocation, images_json, created_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    target,
                    harness,
                    project.to_string_lossy(),
                    session.map(|path| path.to_string_lossy()),
                    prompt_mode(mode),
                    message,
                    display_message,
                    invocation,
                    images_json,
                    now_ms(),
                ],
            )
            .map_err(|error| format!("queue prompt: {error}"))?;
        Ok(self.connection.last_insert_rowid())
    }

    pub(crate) fn queued_prompts(&self) -> Result<Vec<QueuedPrompt>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, target, harness, project, session_path, mode, message,
                        display_message, invocation, images_json
                   FROM outbox WHERE state='queued' ORDER BY id",
            )
            .map_err(|error| format!("prepare prompt queue: {error}"))?;
        statement
            .query_map([], |row| {
                let mode = row.get::<_, String>(5)?;
                let images_json = row.get::<_, String>(9)?;
                let images = serde_json::from_str(&images_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        9,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(QueuedPrompt {
                    id: row.get(0)?,
                    target: row.get(1)?,
                    harness: row.get(2)?,
                    project: PathBuf::from(row.get::<_, String>(3)?),
                    session: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                    mode: parse_prompt_mode(&mode),
                    message: row.get(6)?,
                    display_message: row.get(7)?,
                    invocation: row.get(8)?,
                    images,
                })
            })
            .map_err(|error| format!("query prompt queue: {error}"))?
            .map(|row| row.map_err(|error| format!("decode queued prompt: {error}")))
            .collect()
    }

    pub(crate) fn prompt_presentations(
        &self,
        session: &Path,
    ) -> Result<Vec<PromptPresentation>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT resolved_message, display_message, invocation
                   FROM prompt_presentations
                  WHERE session_path=?1
                  ORDER BY created_ms, id",
            )
            .map_err(|error| format!("prepare prompt presentations: {error}"))?;
        statement
            .query_map([session.to_string_lossy()], |row| {
                Ok(PromptPresentation {
                    resolved_message: row.get(0)?,
                    display_message: row.get(1)?,
                    invocation: row.get(2)?,
                })
            })
            .map_err(|error| format!("query prompt presentations: {error}"))?
            .map(|row| row.map_err(|error| format!("decode prompt presentation: {error}")))
            .collect()
    }

    pub(crate) fn complete_prompt(
        &mut self,
        id: i64,
        target: &str,
        session: Option<&Path>,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("start queued prompt completion {id}: {error}"))?;
        if let Some(draft_id) = target.strip_prefix("draft:").filter(|id| !id.is_empty())
            && let Some(session) = session
        {
            let session = crate::sessions::normalize_session_path(session);
            let app_session_id = transaction
                .query_row(
                    "SELECT app_session_id FROM drafts WHERE id=?1",
                    [draft_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| format!("identify queued prompt {id} draft: {error}"))?;
            if let Some(app_session_id) = app_session_id {
                associate_app_session(&transaction, app_session_id, draft_id, &session).map_err(
                    |error| format!("associate queued prompt {id} application session: {error}"),
                )?;
            }
            transaction
                .execute(
                    "UPDATE drafts SET submitted=1, session_path=?2 WHERE id=?1",
                    params![draft_id, session.to_string_lossy()],
                )
                .map_err(|error| {
                    format!("associate queued prompt {id} with its session: {error}")
                })?;
        }
        if let Some(session) = session {
            let session = crate::sessions::normalize_session_path(session);
            transaction
                .execute(
                    "INSERT INTO prompt_presentations(
                       session_path, resolved_message, display_message, invocation, created_ms
                     )
                     SELECT ?2, message, display_message, invocation, created_ms
                       FROM outbox
                      WHERE id=?1 AND display_message IS NOT NULL AND invocation IS NOT NULL",
                    params![id, session.to_string_lossy()],
                )
                .map_err(|error| format!("save prompt presentation {id}: {error}"))?;
        }
        transaction
            .execute("DELETE FROM outbox WHERE id=?1", [id])
            .map_err(|error| format!("complete queued prompt {id}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit queued prompt completion {id}: {error}"))
    }

    pub(crate) fn begin_prompt(&self, id: i64) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE outbox SET state='sending', error=NULL WHERE id=?1 AND state='queued'",
                [id],
            )
            .map_err(|error| format!("start queued prompt {id}: {error}"))?;
        if changed == 1 {
            Ok(())
        } else {
            Err(format!("queued prompt {id} is no longer ready to send"))
        }
    }

    pub(crate) fn fail_prompt(&self, id: i64, error: &str) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE outbox SET state='failed', error=?2 WHERE id=?1",
                params![id, error],
            )
            .map(|_| ())
            .map_err(|db_error| format!("fail queued prompt {id}: {db_error}"))
    }
}
