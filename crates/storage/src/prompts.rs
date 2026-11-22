use super::*;
use crate::agents::Backend;

#[cfg(test)]
#[path = "prompts_tests.rs"]
mod tests;

pub struct PromptSubmission<'a> {
    pub target: &'a str,
    pub harness: Backend,
    pub project: &'a Path,
    pub session: Option<&'a Path>,
    pub profile_id: Option<&'a str>,
    pub provisional_title: Option<&'a str>,
    pub mode: PromptMode,
    pub message: &'a str,
    pub display_message: Option<&'a str>,
    pub invocation: Option<&'a str>,
    pub images: &'a [PromptImage],
}

pub struct EnqueuedPrompt {
    pub outbox_id: i64,
    pub app_session_id: sessions::AppSessionId,
    pub images: Vec<PromptImage>,
    pub bound_session: Option<sessions::SessionTarget>,
}

#[derive(Default)]
pub struct PromptHistoryOverlay {
    pub stamp: Option<TranscriptStamp>,
    pub accepted: Vec<serde_json::Value>,
    pub presentations: Vec<PromptPresentation>,
}

impl PromptHistoryOverlay {
    pub fn apply(self, messages: &mut Arc<Vec<serde_json::Value>>) {
        use serde_json::Value;
        if !self.accepted.is_empty() {
            let history_was_empty = messages.is_empty();
            let mut submission_ids = messages
                .iter()
                .filter_map(|message| message.get("submissionId").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<std::collections::HashSet<_>>();
            for message in self.accepted {
                let Some(id) = message.get("submissionId").and_then(Value::as_str) else {
                    if history_was_empty {
                        Arc::make_mut(messages).push(message);
                    }
                    continue;
                };
                if !history_was_empty
                    && message.get("deliveryTracked").and_then(Value::as_bool) != Some(true)
                {
                    continue;
                }
                if submission_ids.insert(id.to_owned()) {
                    Arc::make_mut(messages).push(message);
                }
            }
        }
        if !self.presentations.is_empty() {
            farcaster_conversation::annotate_prompt_presentations(
                Arc::make_mut(messages).as_mut_slice(),
                &self.presentations,
            );
        }
    }
}

fn unique_prompt_session(db: &Connection, session: &Path) -> Result<Option<i64>, String> {
    let locator = sessions::normalize_session_path(session);
    db.query_row(
        "SELECT min(id) FROM sessions WHERE locator=?1 HAVING count(*)=1",
        [locator.to_string_lossy()],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| format!("locate unambiguous session for prompt history: {error}"))
}

impl StateStore {
    pub(super) fn pending_prompt_target(
        &self,
        outbox_id: i64,
    ) -> Result<Option<sessions::SessionTarget>, String> {
        self.connection.query_row(
            "SELECT s.harness, s.locator, s.backend_id FROM outbox o JOIN sessions s ON s.id=o.session_id
              WHERE o.id=?1 AND o.state='pending'",
            [outbox_id],
            |row| {
                let harness = super::backend::get(row, 0)?;
                let path: Option<String> = row.get(1)?;
                let id: Option<String> = row.get(2)?;
                Ok(path.map(|path| sessions::SessionTarget {
                    harness,
                    path: sessions::normalize_session_path(Path::new(&path)),
                    id: id.unwrap_or(path),
                }))
            },
        ).map_err(|error| format!("resolve pending prompt {outbox_id}: {error}"))
    }

    pub(super) fn dismiss_prompt_receipt(
        &self,
        session_id: i64,
        receipt_id: &str,
    ) -> Result<(), String> {
        self.connection.execute(
            "INSERT INTO session_events(session_id,seq,t,schema_version,body)
             SELECT s.id, (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=s.id),
                    ?3,1,json_object('type','prompt_delivery_resolution','submissionId',?2,'status','cancelled')
               FROM sessions s WHERE s.id=?1
                AND NOT EXISTS (SELECT 1 FROM session_events e WHERE e.session_id=s.id
                  AND json_extract(e.body,'$.submissionId')=?2
                  AND json_extract(e.body,'$.type') IN ('prompt_delivery_receipt','prompt_delivery_resolution'))",
            params![session_id, receipt_id, now_ms()],
        ).map_err(|error| format!("save pending message dismissal: {error}"))?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_prompt(
        &self,
        target: &str,
        harness: Backend,
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

    #[cfg(any(test, feature = "test-support"))]
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_prompt_with_presentation(
        &self,
        target: &str,
        harness: Backend,
        project: &Path,
        session: Option<&Path>,
        mode: PromptMode,
        message: &str,
        display_message: Option<&str>,
        invocation: Option<&str>,
        images: &[PromptImage],
    ) -> Result<i64, String> {
        self.insert_prompt(PromptSubmission {
            target,
            harness,
            project,
            session,
            profile_id: None,
            provisional_title: None,
            mode,
            message,
            display_message,
            invocation,
            images,
        })
        .map(|prompt| prompt.outbox_id)
    }

    pub(super) fn insert_prompt(
        &self,
        submission: PromptSubmission<'_>,
    ) -> Result<EnqueuedPrompt, String> {
        let PromptSubmission {
            target,
            mode,
            message,
            display_message,
            invocation,
            images,
            ..
        } = submission;
        let (images_json, images) = self.encode_prompt_images(images)?;
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
                .map_err(|error| format!("start prompt enqueue: {error}"))?;
        let (session_id, bound_session) = self.resolve_prompt_session(&transaction, &submission)?;
        let app_session_id = sessions::AppSessionId::try_from(session_id)?;
        let inserted = transaction
            .execute(
                "INSERT INTO outbox(
                   session_id, mode, message, display_message, invocation, images_json,
                   provider, model, effort, service_tier, state, created_ms
                 ) SELECT s.id, ?2, ?3, ?4, ?5, ?6,
                          m.provider, m.model, m.effort, m.service_tier, ?7, ?8
                     FROM sessions s LEFT JOIN session_models m ON m.session_id=s.id
                    WHERE s.id=?1",
                params![
                    session_id,
                    prompt_mode(mode),
                    message,
                    display_message,
                    invocation,
                    images_json,
                    "pending",
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
        Ok(EnqueuedPrompt {
            outbox_id: id,
            app_session_id,
            images,
            bound_session,
        })
    }

    fn resolve_prompt_session(
        &self,
        transaction: &Transaction<'_>,
        submission: &PromptSubmission<'_>,
    ) -> Result<(i64, Option<sessions::SessionTarget>), String> {
        let PromptSubmission {
            target,
            harness,
            session,
            profile_id,
            provisional_title,
            ..
        } = *submission;
        let draft = sessions::draft_id(target).filter(|key| !key.is_empty());
        if let Some(key) = draft {
            let project = sessions::normalize_session_path(submission.project);
            // An editor save can still be queued when the first prompt arrives.
            // After submission or binding, the stored row owns the launch configuration.
            let row = transaction
                .query_row(
                    "SELECT s.id, s.locator, s.backend_id, CASE WHEN p.path=?2 THEN p.id END,
                        (s.submitted=0 AND s.locator IS NULL)
                        OR (p.path=?2 AND s.harness=?3 AND s.profile_id IS ?4)
                   FROM sessions s JOIN projects p ON p.id=s.project_id WHERE s.client_key=?1",
                    params![key, project.to_string_lossy(), harness.as_str(), profile_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, bool>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| format!("resolve submitted draft {key}: {error}"))?;
            if let Some((id, path, native_id, matching_project_id, can_submit)) = row {
                if !can_submit {
                    return Err(format!(
                        "Draft {key} belongs to a different backend, profile, or project"
                    ));
                }
                let project_id = match matching_project_id {
                    Some(id) => id,
                    None => ensure_project(transaction, &project, u64_to_i64(now_ms()))?,
                };
                transaction
                    .execute(
                        "UPDATE sessions SET submitted=1, project_id=?2, harness=?3, profile_id=?4,
                            title=COALESCE(NULLIF(title,''),?5,'') WHERE id=?1",
                        params![
                            id,
                            project_id,
                            harness.as_str(),
                            profile_id,
                            provisional_title
                        ],
                    )
                    .map_err(|error| format!("save submitted draft {key}: {error}"))?;
                let bound_session = path.map(|path| sessions::SessionTarget {
                    harness,
                    path: sessions::normalize_session_path(Path::new(&path)),
                    id: native_id.unwrap_or(path),
                });
                return Ok((id, bound_session));
            }
        }
        // Promotion may detach the draft key while its actor still knows the locator.
        let existing = if let Some(path) = session {
            self.session_id_for_target(&sessions::session_target(path), Some(path), Some(harness))?
        } else if draft.is_none() {
            self.session_id_for_target(target, None, Some(harness))?
        } else {
            None
        };
        let id = match existing {
            Some(id) => id,
            None => super::identity::create_target_session(transaction, submission)?,
        };
        Ok((id, None))
    }

    pub fn queued_prompts(&self) -> Result<Vec<QueuedPrompt>, String> {
        self.prompts_in_state("pending")
    }

    pub fn record_prompt_dispatch(&self, outbox_id: i64, receipt_id: &str) -> Result<(), String> {
        self.connection.execute(
            "INSERT INTO session_events(session_id,seq,t,schema_version,body)
             SELECT o.session_id,
                    (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=o.session_id),
                    ?3,1,json_object('type','prompt_dispatch','outboxId',o.id,'submissionId',?2)
               FROM outbox o WHERE o.id=?1 AND o.state='pending'",
            params![outbox_id, receipt_id, now_ms()],
        ).map_err(|error| format!("save prompt dispatch {outbox_id}: {error}"))?;
        Ok(())
    }

    pub fn cancel_queued_prompts(&self, ids: &[i64]) -> Result<(), String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| format!("start queued prompt cancellation: {error}"))?;
        for &id in ids {
            transaction
                .execute(
                    "UPDATE outbox
                        SET state='cancelled', error=NULL
                      WHERE id=?1 AND state='pending'",
                    [id],
                )
                .map_err(|error| format!("cancel queued prompt {id}: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit queued prompt cancellation: {error}"))
    }

    fn prompts_in_state(&self, state: &str) -> Result<Vec<QueuedPrompt>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT o.id, s.client_key, s.locator, s.harness, p.path, o.mode, o.message,
                        o.display_message, o.invocation, o.images_json, s.backend_id
                   FROM outbox o
                   JOIN sessions s ON s.id = o.session_id
                   JOIN projects p ON p.id = s.project_id
                  WHERE o.state=?1 ORDER BY o.id",
            )
            .map_err(|error| format!("prepare prompt queue: {error}"))?;
        statement
            .query_map([state], |row| {
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
                    submission_id: None,
                    target,
                    harness: super::backend::get(row, 3)?,
                    project: crate::sessions::normalize_session_path(Path::new(&project)),
                    backend_session_id: row.get(10)?,
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

    pub fn prompt_presentations(&self, session: &Path) -> Result<Vec<PromptPresentation>, String> {
        self.presentations_for_id(unique_prompt_session(&self.connection, session)?)
    }

    fn presentations_for_id(
        &self,
        session_id: Option<i64>,
    ) -> Result<Vec<PromptPresentation>, String> {
        let mut statement = self
            .connection
            .prepare(
                // Before presentation events carried an id, the delivery transaction
                // wrote its receipt in the following slot. Verify its outbox payload
                // before using that relationship for older databases.
                "SELECT e.body, COALESCE(json_extract(e.body,'$.submissionId'), (
                    SELECT json_extract(r.body,'$.submissionId')
                    FROM session_events r JOIN session_events a ON a.session_id=e.session_id
                      AND json_extract(a.body,'$.type')='accepted_prompt'
                      AND json_extract(a.body,'$.submissionId')=json_extract(r.body,'$.submissionId')
                    JOIN outbox o ON o.id=json_extract(a.body,'$.outboxId') AND o.session_id=e.session_id
                    WHERE r.session_id=e.session_id AND r.seq=e.seq+1
                      AND json_extract(r.body,'$.type')='prompt_delivery_receipt'
                      AND o.message=json_extract(e.body,'$.resolved')
                      AND o.display_message=json_extract(e.body,'$.display')
                      AND o.invocation=json_extract(e.body,'$.invocation')
                )) FROM session_events e
                  WHERE e.session_id=?1 AND json_extract(e.body, '$.type')='prompt_presentation'
                  ORDER BY e.seq",
            )
            .map_err(|error| format!("prepare prompt presentations: {error}"))?;
        statement
            .query_map([session_id], |row| {
                let body = row.get::<_, String>(0)?;
                let value: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(PromptPresentation {
                    submission_id: row.get(1)?,
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

    /// Displaying a page on its own is safe when full-history projection cannot
    /// append a local delivery or substitute a stored prompt presentation.
    pub fn can_display_history_tail(&self, session: &Path) -> Result<bool, String> {
        self.can_display_tail_for_id(unique_prompt_session(&self.connection, session)?)
    }

    pub(super) fn can_display_tail_for_id(&self, session_id: Option<i64>) -> Result<bool, String> {
        self.connection.query_row(
            "SELECT NOT EXISTS (
                SELECT 1 FROM session_events e
                WHERE e.session_id=?1 AND (
                    json_extract(e.body,'$.type')='prompt_presentation' OR (
                        json_extract(e.body,'$.type')='accepted_prompt'
                        AND json_extract(e.body,'$.deliveryTracked')=1
                        AND COALESCE(json_extract(e.body,'$.deliveryStatus'),'accepted')='accepted'
                        AND NOT EXISTS (
                            SELECT 1 FROM session_events r WHERE r.session_id=e.session_id
                            AND json_extract(r.body,'$.type') IN ('prompt_delivery_receipt','prompt_delivery_resolution')
                            AND json_extract(r.body,'$.submissionId')=json_extract(e.body,'$.submissionId')
                        )
                    )
                )
            )",
            [session_id], |row| row.get(0)
        ).map_err(|error| format!("check local history presentation: {error}"))
    }

    pub fn accepted_prompt_history(
        &self,
        session: &Path,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.accepted_history_for_id(unique_prompt_session(&self.connection, session)?)
    }

    fn accepted_history_for_id(
        &self,
        session_id: Option<i64>,
    ) -> Result<Vec<serde_json::Value>, String> {
        #[derive(serde::Deserialize)]
        struct AcceptedPrompt {
            #[serde(rename = "submissionId", default)]
            submission_id: Option<String>,
            #[serde(rename = "deliveryStatus", default)]
            delivery_status: Option<String>,
            #[serde(rename = "promptMode", default)]
            prompt_mode: Option<String>,
            #[serde(rename = "deliveryTracked", default)]
            delivery_tracked: bool,
            message: String,
            images: Vec<super::images::StoredImage>,
        }

        let mut statement = self
            .connection
            .prepare(
                "SELECT e.body FROM session_events e
                  WHERE e.session_id=?1 AND json_extract(e.body,'$.type')='accepted_prompt'
                    AND COALESCE(json_extract(e.body,'$.deliveryStatus'),'accepted')='accepted'
                     AND NOT EXISTS (
                        SELECT 1 FROM session_events resolution
                         WHERE resolution.session_id=e.session_id
                           AND json_extract(resolution.body,'$.type') IN
                               ('prompt_delivery_receipt','prompt_delivery_resolution')
                           AND json_extract(resolution.body,'$.submissionId')=
                               json_extract(e.body,'$.submissionId')
                     )
                  ORDER BY e.seq",
            )
            .map_err(|error| format!("read accepted prompts: {error}"))?;
        let rows = statement
            .query_map([session_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query accepted prompts: {error}"))?;
        rows.map(|row| {
            let body = row.map_err(|error| error.to_string())?;
            let prompt: AcceptedPrompt = serde_json::from_str(&body)
                .map_err(|error| format!("decode accepted prompt: {error}"))?;
            let mut content = vec![serde_json::json!({"type":"text", "text":prompt.message})];
            for image in prompt.images {
                let image = self.decode_prompt_image(image)?.into_inline()?;
                content.push(serde_json::json!({
                    "type": "image",
                    "data": image.data,
                    "mimeType": image.mime_type,
                }));
            }
            let mut history = serde_json::json!({"role":"user", "content":content});
            if let Some(submission_id) = prompt.submission_id {
                history["submissionId"] = submission_id.into();
                history["deliveryStatus"] = prompt
                    .delivery_status
                    .unwrap_or_else(|| "accepted".into())
                    .into();
            }
            if let Some(prompt_mode) = prompt.prompt_mode {
                history["queued"] = (prompt_mode != "normal").into();
                history["promptMode"] = prompt_mode.into();
            }
            history["deliveryTracked"] = prompt.delivery_tracked.into();
            Ok(history)
        })
        .collect()
    }

    pub fn reconcile_prompt_deliveries(
        &mut self,
        session: &Path,
        evidence: &crate::sessions::PromptDeliveryReconciliation,
    ) -> Result<(), String> {
        let id = unique_prompt_session(&self.connection, session)?;
        self.reconcile_deliveries_for_id(id, evidence)
    }

    pub(super) fn transcript_prompt_overlay(
        &mut self,
        session: TranscriptSession<'_>,
        evidence: Option<&sessions::PromptDeliveryReconciliation>,
    ) -> Result<PromptHistoryOverlay, String> {
        let id = session.find(&self.connection).map_err(|e| e.to_string())?;
        if let Some(evidence) = evidence {
            self.reconcile_deliveries_for_id(id, evidence)?;
        }
        // Read the overlay and its version from one SQLite snapshot.
        let _read = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let mut overlay = self.prompt_overlay_for_id(id)?;
        overlay.stamp = match id {
            Some(id) => self
                .transcript_stamp_for_id(id)
                .map_err(|e| e.to_string())?,
            None => None,
        };
        Ok(overlay)
    }

    pub(super) fn prompt_overlay_for_id(
        &self,
        id: Option<i64>,
    ) -> Result<PromptHistoryOverlay, String> {
        Ok(PromptHistoryOverlay {
            stamp: None,
            accepted: self.accepted_history_for_id(id)?,
            presentations: self.presentations_for_id(id)?,
        })
    }

    fn reconcile_deliveries_for_id(
        &mut self,
        session_id: Option<i64>,
        evidence: &sessions::PromptDeliveryReconciliation,
    ) -> Result<(), String> {
        let Some(session_id) = session_id else {
            return Ok(());
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start prompt delivery reconciliation: {error}"))?;
        let unresolved = {
            let mut statement = transaction
                .prepare(
                    "SELECT DISTINCT json_extract(e.body,'$.submissionId')
                       FROM session_events e
                      WHERE e.session_id=?1
                        AND json_extract(e.body,'$.type')='accepted_prompt'
                        AND COALESCE(json_extract(e.body,'$.deliveryStatus'),'accepted')='accepted'
                        AND json_extract(e.body,'$.submissionId') IS NOT NULL
                        AND NOT EXISTS (
                            SELECT 1 FROM session_events resolution
                             WHERE resolution.session_id=e.session_id
                               AND json_extract(resolution.body,'$.type') IN
                                   ('prompt_delivery_receipt','prompt_delivery_resolution')
                               AND json_extract(resolution.body,'$.submissionId')=
                                   json_extract(e.body,'$.submissionId')
                        )",
                )
                .map_err(|error| format!("read unresolved prompt deliveries: {error}"))?;
            statement
                .query_map([session_id], |row| row.get::<_, String>(0))
                .map_err(|error| format!("query unresolved prompt deliveries: {error}"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| format!("decode unresolved prompt delivery: {error}"))?
        };
        let delivered = evidence
            .delivered
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let pending = evidence
            .pending
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for submission_id in &delivered {
            let outbox_ids = {
                let mut statement = transaction
                    .prepare(
                        "SELECT id FROM outbox
                          WHERE session_id=?1 AND state='pending' AND id IN (
                            SELECT json_extract(body,'$.outboxId') FROM session_events
                             WHERE session_id=?1 AND json_extract(body,'$.type')='prompt_dispatch'
                               AND json_extract(body,'$.submissionId')=?2
                          )",
                    )
                    .map_err(|error| format!("find native prompt delivery: {error}"))?;
                statement
                    .query_map(params![session_id, submission_id], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(|error| format!("query native prompt delivery: {error}"))?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|error| format!("decode native prompt delivery: {error}"))?
            };
            for id in outbox_ids {
                record_prompt_delivery(&transaction, id, submission_id, true, true)?;
            }
        }
        for submission_id in unresolved {
            let (event_type, status) = if delivered.contains(submission_id.as_str()) {
                ("prompt_delivery_receipt", "delivered")
            } else if pending.contains(submission_id.as_str()) {
                continue;
            } else if evidence.absence_is_not_delivered {
                ("prompt_delivery_resolution", "not_delivered")
            } else {
                continue;
            };
            transaction
                .execute(
                    "INSERT INTO session_events(session_id,seq,t,schema_version,body)
                     SELECT ?1,
                            (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=?1),
                            ?2,1,
                            json_object('type',?3,'submissionId',?4,'status',?5)
                      WHERE NOT EXISTS (
                        SELECT 1 FROM session_events WHERE session_id=?1
                          AND json_extract(body,'$.submissionId')=?4
                          AND json_extract(body,'$.type') IN
                              ('prompt_delivery_receipt','prompt_delivery_resolution')
                      )",
                    params![session_id, now_ms(), event_type, submission_id, status],
                )
                .map_err(|error| {
                    format!("save prompt delivery resolution {submission_id}: {error}")
                })?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit prompt delivery reconciliation: {error}"))
    }

    pub fn record_prompt_acceptance(
        &mut self,
        id: i64,
        target: &str,
        session: Option<&Path>,
        receipt_id: &str,
        delivery_tracked: bool,
    ) -> Result<(), String> {
        self.record_prompt_delivery(id, target, session, receipt_id, delivery_tracked, false)
    }

    pub fn complete_delivered_prompt(
        &mut self,
        id: i64,
        target: &str,
        session: Option<&Path>,
        receipt_id: &str,
        delivery_tracked: bool,
    ) -> Result<(), String> {
        self.record_prompt_delivery(id, target, session, receipt_id, delivery_tracked, true)
    }

    fn record_prompt_delivery(
        &mut self,
        id: i64,
        _target: &str,
        session: Option<&Path>,
        receipt_id: &str,
        delivery_tracked: bool,
        delivered: bool,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start prompt delivery record {id}: {error}"))?;
        let draft_id = transaction
            .query_row(
                "SELECT s.client_key FROM outbox o JOIN sessions s ON s.id=o.session_id
              WHERE o.id=?1 AND o.state='pending'",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| format!("identify queued prompt {id}: {error}"))?;
        let Some(draft_id) = draft_id else {
            return transaction
                .commit()
                .map_err(|error| format!("finish duplicate prompt delivery: {error}"));
        };
        if let Some(draft_id) = draft_id
            && let Some(session) = session
        {
            let session = crate::sessions::normalize_session_path(session);
            bind_locator(&transaction, &draft_id, &session).map_err(|error| {
                format!("associate queued prompt {id} with its session: {error}")
            })?;
        }
        record_prompt_delivery(&transaction, id, receipt_id, delivery_tracked, delivered)?;
        transaction
            .commit()
            .map_err(|error| format!("commit prompt delivery record {id}: {error}"))
    }

    pub fn record_prompt_receipt_delivered(
        &mut self,
        receipt_id: &str,
        outbox_id: Option<i64>,
    ) -> Result<(), String> {
        if receipt_id.is_empty() {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start prompt delivery receipt: {error}"))?;
        let outbox_id = match outbox_id {
            Some(id) => Some(id),
            None => transaction
                .query_row(
                    "SELECT min(json_extract(body,'$.outboxId')) FROM session_events
                  WHERE json_extract(body,'$.submissionId')=?1
                    AND json_extract(body,'$.type') IN ('prompt_dispatch','accepted_prompt')
                    AND json_extract(body,'$.outboxId') IS NOT NULL
                  HAVING COUNT(DISTINCT json_extract(body,'$.outboxId'))=1",
                    [receipt_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| {
                    format!("find outbox for delivery receipt {receipt_id}: {error}")
                })?,
        };
        let session_id = transaction
            .query_row(
                "SELECT session_id FROM outbox WHERE id=?2
                 UNION ALL
                 SELECT min(session_id) FROM session_events
                  WHERE json_extract(body,'$.type')='accepted_prompt'
                    AND json_extract(body,'$.submissionId')=?1
                    AND (?2 IS NULL OR json_extract(body,'$.outboxId')=?2)
                 HAVING count(DISTINCT session_id)=1",
                rusqlite::params![receipt_id, outbox_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("locate prompt delivery receipt {receipt_id}: {error}"))?;
        let Some(session_id) = session_id else {
            return transaction
                .commit()
                .map_err(|error| format!("finish unmatched prompt delivery receipt: {error}"));
        };
        if let Some(outbox_id) = outbox_id {
            record_prompt_delivery(&transaction, outbox_id, receipt_id, true, true)?;
        }
        transaction
            .execute(
                "INSERT INTO session_events(session_id, seq, t, schema_version, body)
                 SELECT ?1,
                        (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=?1),
                        ?2, 1,
                        json_object('type','prompt_delivery_receipt','submissionId',?3)
                  WHERE NOT EXISTS (
                      SELECT 1 FROM session_events
                       WHERE session_id=?1
                         AND json_extract(body,'$.type')='prompt_delivery_receipt'
                         AND json_extract(body,'$.submissionId')=?3
                  )",
                rusqlite::params![session_id, now_ms(), receipt_id],
            )
            .map_err(|error| format!("save prompt delivery receipt {receipt_id}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit prompt delivery receipt {receipt_id}: {error}"))
    }
}
impl StateStore {
    pub fn begin_prompt(&self, id: i64) -> Result<(), String> {
        let pending = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outbox WHERE id=?1 AND state='pending')",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("check queued prompt {id}: {error}"))?;
        if pending {
            Ok(())
        } else {
            Err(format!("queued prompt {id} is no longer ready to send"))
        }
    }
}

fn record_prompt_delivery(
    transaction: &Transaction<'_>,
    id: i64,
    receipt_id: &str,
    delivery_tracked: bool,
    delivered: bool,
) -> Result<(), String> {
    let pending = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE id=?1 AND state='pending')",
            [id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("check prompt delivery {id}: {error}"))?;
    if !pending {
        return Ok(());
    }
    transaction
        .execute(
            "UPDATE sessions SET submitted=1 WHERE id=(SELECT session_id FROM outbox WHERE id=?1)",
            [id],
        )
        .map_err(|error| format!("record prompt acceptance: {error}"))?;
    transaction.execute(
            "INSERT INTO session_events(session_id, seq, t, schema_version, body)
             SELECT o.session_id,
                    (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=o.session_id),
                    o.created_ms, 1,
                    json_object('type','accepted_prompt','submissionId',?2,'outboxId',o.id,
                                'deliveryStatus','accepted',
                                'deliveryTracked',json(CASE WHEN ?3 THEN 'true' ELSE 'false' END),
                                'promptMode',o.mode,'message',o.message,
                                'images',json(o.images_json))
               FROM outbox o WHERE o.id=?1
                AND NOT EXISTS (
                    SELECT 1 FROM session_events e
                     WHERE e.session_id=o.session_id
                       AND json_extract(e.body,'$.type')='accepted_prompt'
                       AND json_extract(e.body,'$.submissionId')=?2
                )",
            rusqlite::params![id, receipt_id, delivery_tracked],
        ).map_err(|error| format!("save accepted prompt {id}: {error}"))?;
    if delivered {
        transaction.execute(
                "INSERT INTO session_events(session_id, seq, t, schema_version, body)
                 SELECT o.session_id,
                        (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=o.session_id),
                        o.created_ms, 1,
                        json_object('type','prompt_presentation','submissionId',?2,'resolved',o.message,
                                    'display',o.display_message,'invocation',o.invocation)
                   FROM outbox o
                  WHERE o.id=?1 AND o.display_message IS NOT NULL AND o.invocation IS NOT NULL",
                params![id, receipt_id],
            ).map_err(|error| format!("save prompt presentation {id}: {error}"))?;
        transaction.execute(
                "INSERT INTO session_events(session_id, seq, t, schema_version, body)
                 SELECT o.session_id,
                        (SELECT COALESCE(MAX(seq),0)+1 FROM session_events WHERE session_id=o.session_id),
                        ?3, 1,
                        json_object('type','prompt_delivery_receipt','submissionId',?2)
                   FROM outbox o WHERE o.id=?1
                    AND NOT EXISTS (
                        SELECT 1 FROM session_events e WHERE e.session_id=o.session_id
                         AND json_extract(e.body,'$.type')='prompt_delivery_receipt'
                         AND json_extract(e.body,'$.submissionId')=?2
                    )",
                rusqlite::params![id, receipt_id, now_ms()],
            ).map_err(|error| format!("save delivered prompt {id}: {error}"))?;
        transaction
            .execute(
                "UPDATE outbox SET state='acked', error=NULL WHERE id=?1 AND state='pending'",
                [id],
            )
            .map_err(|error| format!("complete delivered prompt {id}: {error}"))?;
    }
    Ok(())
}
