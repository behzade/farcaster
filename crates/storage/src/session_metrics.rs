use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutcome {
    Done,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionFacts {
    pub metrics: serde_json::Value,
    pub goal: Option<agents::SessionGoal>,
    pub outcome: Option<SessionOutcome>,
    pub cache_usage: farcaster_agent_protocol::CacheUsage,
    pub service_tier: Option<String>,
}

impl StateStore {
    #[cfg(any(test, feature = "test-support"))]
    pub fn save_session_facts(
        &mut self,
        harness: Backend,
        project: &Path,
        session: &Path,
        facts: &SessionFacts,
    ) -> Result<(), String> {
        self.record_session_facts(
            ObservedSession {
                harness,
                project,
                path: session,
                profile_id: None,
                draft: None,
            },
            facts,
        )
    }

    pub(super) fn record_session_facts(
        &mut self,
        session: ObservedSession<'_>,
        facts: &SessionFacts,
    ) -> Result<(), String> {
        let mut metrics: serde_json::Map<String, serde_json::Value> =
            ["tokens", "totalCost", "contextUsage"]
                .into_iter()
                .filter_map(|key| {
                    facts
                        .metrics
                        .get(key)
                        .filter(|value| !value.is_null())
                        .map(|value| (key.to_owned(), value.clone()))
                })
                .collect();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session metrics save: {error}"))?;
        let now = u64_to_i64(now_ms());
        let id = if let Some(draft) = session.draft {
            draft_storage::bind_prompt_locator(
                &tx,
                draft,
                session.harness,
                session.project,
                session.profile_id,
                session.path,
            )?
        } else {
            let locator = sessions::normalize_session_path(session.path);
            let project_id = ensure_project(&tx, session.project, now)?;
            ensure_locator_session(
                &tx,
                session.harness,
                &locator.to_string_lossy(),
                project_id,
                session.project,
            )?
        };
        let previous: Option<String> = tx
            .query_row(
                "SELECT body FROM session_events WHERE session_id=?1
             AND json_extract(body,'$.type')='session_metrics' ORDER BY t DESC,seq DESC LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("read prior session metrics: {error}"))?;
        if let Some(previous) = &previous {
            let previous: serde_json::Value = serde_json::from_str(previous)
                .map_err(|error| format!("decode prior session metrics: {error}"))?;
            if let Some(previous) = previous["metrics"].as_object() {
                for (key, value) in previous {
                    metrics.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        let mut record = serde_json::to_value(facts)
            .map_err(|error| format!("encode session facts: {error}"))?;
        record["type"] = "session_metrics".into();
        record["metrics"] = metrics.into();
        let body = record.to_string();
        if previous.as_deref() != Some(&body) {
            tx.execute(
                "INSERT INTO session_events(session_id,seq,t,schema_version,body)
                 SELECT ?1,COALESCE(MAX(seq),0)+1,?2,1,?3 FROM session_events WHERE session_id=?1",
                params![id, now, body],
            )
            .map_err(|error| format!("save session metrics: {error}"))?;
        }
        tx.commit()
            .map_err(|error| format!("commit session metrics: {error}"))
    }

    pub fn load_session_facts(
        &self,
        harness: Backend,
        session: &Path,
    ) -> Result<SessionFacts, String> {
        let locator = sessions::normalize_session_path(session);
        let body: Option<String> = self
            .connection
            .query_row(
                "SELECT body FROM session_events
             WHERE session_id=(SELECT id FROM sessions WHERE harness=?1 AND locator=?2)
               AND json_extract(body,'$.type')='session_metrics'
             ORDER BY t DESC,seq DESC LIMIT 1",
                params![harness.as_str(), locator.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("load session metrics: {error}"))?;
        let record: serde_json::Value = body
            .map(|body| serde_json::from_str(&body))
            .transpose()
            .map_err(|error| format!("decode session facts: {error}"))?
            .unwrap_or_else(|| serde_json::json!({}));
        let mut facts: SessionFacts = serde_json::from_value(record.clone())
            .map_err(|error| format!("decode session facts: {error}"))?;
        // Older records saved this observation only in session_models.
        if record.get("serviceTier").is_none() {
            facts.service_tier = self.connection.query_row(
                "SELECT m.service_tier FROM session_models m JOIN sessions s ON s.id=m.session_id
                 WHERE s.harness=?1 AND s.locator=?2",
                params![harness.as_str(), locator.to_string_lossy()],
                |row| row.get(0),
            ).optional().map_err(|error| format!("load observed service tier: {error}"))?.flatten();
        }
        Ok(facts)
    }
}

#[cfg(test)]
#[path = "session_metrics_tests.rs"]
mod tests;
