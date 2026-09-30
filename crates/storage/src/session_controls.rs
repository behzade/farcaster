use super::*;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "control", content = "value", rename_all = "snake_case")]
pub enum SessionControlSelection {
    Model(crate::protocol::Model),
    Effort(Option<String>),
    ServiceTier(Option<String>),
}

impl StateStore {
    pub fn save_session_control_selection(
        &mut self,
        harness: Backend,
        project: &Path,
        session: &Path,
        selection: &SessionControlSelection,
    ) -> Result<(), String> {
        let locator = sessions::normalize_session_path(session);
        let mut body = serde_json::to_value(selection)
            .map_err(|error| format!("encode session selection: {error}"))?;
        body["type"] = "session_control_selection".into();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("start session selection save: {error}"))?;
        let now = u64_to_i64(now_ms());
        let project_id = ensure_project(&tx, project, now)?;
        let id = ensure_locator_session(
            &tx,
            harness,
            &locator.to_string_lossy(),
            project_id,
            project,
        )?;
        tx.execute(
            "INSERT INTO session_events(session_id,seq,t,schema_version,body)
             SELECT ?1,COALESCE(MAX(seq),0)+1,?2,1,?3 FROM session_events WHERE session_id=?1",
            params![id, now, body.to_string()],
        )
        .map_err(|error| format!("save session selection: {error}"))?;
        tx.commit()
            .map_err(|error| format!("commit session selection: {error}"))
    }

    pub fn load_session_control_selections(
        &self,
        harness: Backend,
        session: &Path,
    ) -> Result<Vec<SessionControlSelection>, String> {
        let locator = sessions::normalize_session_path(session);
        let mut statement = self
            .connection
            .prepare(
                "SELECT body FROM (
               SELECT body, ROW_NUMBER() OVER (
                 PARTITION BY json_extract(body,'$.control') ORDER BY t DESC,seq DESC
               ) AS rank FROM session_events
               WHERE session_id=(SELECT id FROM sessions WHERE harness=?1 AND locator=?2)
                 AND json_extract(body,'$.type')='session_control_selection'
             ) WHERE rank=1
             ORDER BY CASE json_extract(body,'$.control')
               WHEN 'model' THEN 0 WHEN 'effort' THEN 1 ELSE 2 END",
            )
            .map_err(|error| format!("prepare session selections: {error}"))?;
        let rows = statement
            .query_map(
                params![harness.as_str(), locator.to_string_lossy()],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("read session selections: {error}"))?;
        rows.map(|row| {
            let body = row.map_err(|error| format!("read session selection: {error}"))?;
            serde_json::from_str(&body)
                .map_err(|error| format!("decode session selection: {error}"))
        })
        .collect()
    }
}

#[cfg(test)]
#[path = "session_controls_tests.rs"]
mod tests;
