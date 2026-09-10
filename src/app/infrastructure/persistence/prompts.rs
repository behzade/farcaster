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
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
                .map_err(|error| format!("start prompt enqueue: {error}"))?;
        let session_id = match self.session_id_for_target(target, session, Some(harness))? {
            Some(id) => id,
            None => super::identity::create_target_session(
                &transaction,
                target,
                harness,
                project,
                session,
            )?,
        };
        let images_json = self.encode_prompt_images(images)?;
        let inserted = self
            .connection
            .execute(
                "INSERT INTO outbox(
                   session_id, mode, message, display_message, invocation, images_json,
                   provider, model, effort, service_tier, created_ms
                 ) SELECT s.id, ?2, ?3, ?4, ?5, ?6,
                          m.provider, m.model, m.effort, m.service_tier, ?7
                     FROM sessions s LEFT JOIN session_models m ON m.session_id=s.id
                    WHERE s.id=?1",
                params![
                    session_id,
                    prompt_mode(mode),
                    message,
                    display_message,
                    invocation,
                    images_json,
                    now_ms(),
                ],
            )
            .map_err(|error| format!("queue prompt: {error}"))?;
        if inserted != 1 {
            return Err(format!(
                "session disappeared before queuing prompt for {target}"
            ));
        }
        let id = transaction.last_insert_rowid();
        transaction
            .commit()
            .map_err(|error| format!("commit prompt enqueue: {error}"))?;
        Ok(id)
    }

    pub(crate) fn queued_prompts(&self) -> Result<Vec<QueuedPrompt>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT o.id, s.client_key, s.locator, s.harness, p.path, o.mode, o.message,
                        o.display_message, o.invocation, o.images_json
                   FROM outbox o
                   JOIN sessions s ON s.id = o.session_id
                   JOIN projects p ON p.id = s.project_id
                  WHERE o.state='queued' ORDER BY o.id",
            )
            .map_err(|error| format!("prepare prompt queue: {error}"))?;
        statement
            .query_map([], |row| {
                let mode = row.get::<_, String>(5)?;
                let images_json = row.get::<_, String>(9)?;
                let images = self.decode_prompt_images(&images_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        9,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::other(error)),
                    )
                })?;
                let client_key = row.get::<_, Option<String>>(1)?;
                let locator = row.get::<_, Option<String>>(2)?;
                let project = row.get::<_, String>(4)?;
                let target = target_for_session(client_key.as_deref(), locator.as_deref())?;
                Ok(QueuedPrompt {
                    id: row.get(0)?,
                    target,
                    harness: row.get(3)?,
                    project: crate::sessions::normalize_session_path(Path::new(&project)),
                    session: locator
                        .map(PathBuf::from)
                        .map(|path| crate::sessions::normalize_session_path(&path)),
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
        let locator = crate::sessions::normalize_session_path(session);
        let mut statement = self
            .connection
            .prepare(
                "SELECT e.body FROM session_events e
                   JOIN sessions s ON s.id = e.session_id
                  WHERE s.locator=?1 AND json_extract(e.body, '$.type')='prompt_presentation'
                  ORDER BY e.seq",
            )
            .map_err(|error| format!("prepare prompt presentations: {error}"))?;
        statement
            .query_map([locator.to_string_lossy()], |row| {
                let body = row.get::<_, String>(0)?;
                let value: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(PromptPresentation {
                    resolved_message: value
                        .get("resolved")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    display_message: value
                        .get("display")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    invocation: value
                        .get("invocation")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .map_err(|error| format!("query prompt presentations: {error}"))?
            .map(|row| row.map_err(|error| format!("decode prompt presentation: {error}")))
            .collect()
    }

    pub(crate) fn accepted_prompt_history(
        &self,
        session: &Path,
    ) -> Result<Vec<serde_json::Value>, String> {
        #[derive(serde::Deserialize)]
        struct AcceptedPrompt {
            message: String,
            images: Vec<super::images::StoredImage>,
        }

        let locator = crate::sessions::normalize_session_path(session);
        let mut statement = self
            .connection
            .prepare(
                "SELECT e.body FROM session_events e JOIN sessions s ON s.id=e.session_id
                  WHERE s.locator=?1 AND json_extract(e.body,'$.type')='accepted_prompt'
                  ORDER BY e.seq",
            )
            .map_err(|error| format!("read accepted prompts: {error}"))?;
        let rows = statement
            .query_map([locator.to_string_lossy()], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query accepted prompts: {error}"))?;
        rows.map(|row| {
            let body = row.map_err(|error| error.to_string())?;
            let prompt: AcceptedPrompt = serde_json::from_str(&body)
                .map_err(|error| format!("decode accepted prompt: {error}"))?;
            let mut content = vec![serde_json::json!({"type":"text", "text":prompt.message})];
            for image in prompt.images {
                content.push(serde_json::json!(
                    self.decode_prompt_image(image)?.into_inline()?
                ));
            }
            Ok(serde_json::json!({"role":"user", "content":content}))
        })
        .collect()
    }

    pub(crate) fn complete_prompt(
        &mut self,
        id: i64,
        _target: &str,
        session: Option<&Path>,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start queued prompt completion {id}: {error}"))?;
        let draft_id = transaction.query_row(
            "SELECT s.client_key FROM outbox o JOIN sessions s ON s.id=o.session_id WHERE o.id=?1",
            [id], |row| row.get::<_, Option<String>>(0),
        ).optional().map_err(|error| format!("identify queued prompt {id}: {error}"))?;
        let Some(draft_id) = draft_id else {
            return transaction
                .commit()
                .map_err(|error| format!("finish duplicate prompt acknowledgement: {error}"));
        };
        if let Some(draft_id) = draft_id
            && let Some(session) = session
        {
            let session = crate::sessions::normalize_session_path(session);
            bind_locator(&transaction, &draft_id, &session).map_err(|error| {
                format!("associate queued prompt {id} with its session: {error}")
            })?;
        }
        transaction.execute(
            "UPDATE sessions SET submitted=1 WHERE id=(SELECT session_id FROM outbox WHERE id=?1)",
            [id],
        ).map_err(|error| format!("record prompt acceptance: {error}"))?;
        transaction.execute(
            "INSERT INTO session_events(session_id, seq, t, schema_version, body)
             SELECT o.session_id,
                    (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=o.session_id),
                    o.created_ms, 1,
                    json_object('type','prompt_presentation','resolved',o.message,
                                'display',o.display_message,'invocation',o.invocation)
               FROM outbox o
              WHERE o.id=?1 AND o.display_message IS NOT NULL AND o.invocation IS NOT NULL",
            [id],
        ).map_err(|error| format!("save prompt presentation {id}: {error}"))?;
        // Transport acceptance can precede durable backend history. Keep the payload
        // before removing it from the delivery queue, including image-only prompts.
        transaction.execute(
            "INSERT INTO session_events(session_id, seq, t, schema_version, body)
             SELECT o.session_id,
                    (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=o.session_id),
                    o.created_ms, 1,
                    json_object('type','accepted_prompt','message',o.message,
                                'images',json(o.images_json))
               FROM outbox o WHERE o.id=?1",
            [id],
        ).map_err(|error| format!("save accepted prompt {id}: {error}"))?;
        transaction
            .execute("DELETE FROM outbox WHERE id=?1", [id])
            .map_err(|error| format!("complete queued prompt {id}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit queued prompt completion {id}: {error}"))
    }
}
impl StateStore {
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
