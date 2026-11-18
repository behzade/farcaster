use super::*;

impl StateStore {
    /// Retain reported session metrics independently of the backend process.
    pub fn save_session_metrics(
        &mut self,
        harness: Backend,
        project: &Path,
        session: &Path,
        stats: &serde_json::Value,
    ) -> Result<(), String> {
        let mut metrics: serde_json::Map<String, serde_json::Value> =
            ["tokens", "totalCost", "contextUsage"]
                .into_iter()
                .filter_map(|key| {
                    stats
                        .get(key)
                        .filter(|value| !value.is_null())
                        .map(|value| (key.to_owned(), value.clone()))
                })
                .collect();
        if metrics.is_empty() {
            return Ok(());
        }
        let locator = sessions::normalize_session_path(session);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session metrics save: {error}"))?;
        let now = u64_to_i64(now_ms());
        let project_id = ensure_project(&tx, project, now)?;
        let id = ensure_locator_session(
            &tx,
            harness,
            &locator.to_string_lossy(),
            project_id,
            project,
        )?;
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
        let body = serde_json::json!({"type":"session_metrics", "metrics":metrics}).to_string();
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

    pub fn load_session_metrics(
        &self,
        harness: Backend,
        session: &Path,
    ) -> Result<serde_json::Value, String> {
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
        let Some(body) = body else {
            return Ok(serde_json::Value::Null);
        };
        let mut record: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| format!("decode session metrics: {error}"))?;
        Ok(record["metrics"].take())
    }
}

#[cfg(test)]
#[path = "session_metrics_tests.rs"]
mod tests;
