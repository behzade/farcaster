use super::*;

impl StateStore {
    pub(crate) fn load_composer_sessions(&self) -> Result<Vec<ComposerRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.id, s.client_key, s.locator, c.text, c.cursor, c.selection_start,
                        c.selection_end, c.history_json
                   FROM composer_sessions c
                   JOIN sessions s ON s.id = c.session_id",
            )
            .map_err(|error| format!("prepare composer sessions: {error}"))?;
        statement
            .query_map([], |row| {
                let history_json = row.get::<_, String>(7)?;
                let history = serde_json::from_str(&history_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                let client_key = row.get::<_, Option<String>>(1)?;
                let locator = row.get::<_, Option<String>>(2)?;
                Ok(ComposerRecord {
                    target: target_for_session(client_key.as_deref(), locator.as_deref())?,
                    text: row.get(3)?,
                    cursor: row.get::<_, u64>(4)?.try_into().unwrap_or(usize::MAX),
                    selection_start: row.get::<_, u64>(5)?.try_into().unwrap_or(usize::MAX),
                    selection_end: row.get::<_, u64>(6)?.try_into().unwrap_or(usize::MAX),
                    history,
                })
            })
            .map_err(|error| format!("query composer sessions: {error}"))?
            .map(|row| row.map_err(|error| format!("decode composer session: {error}")))
            .collect()
    }

    pub(crate) fn save_composer_session(&self, record: &ComposerRecord) -> Result<(), String> {
        let Some(session_id) = self.session_id_for_target(&record.target, None, None)? else {
            return Ok(());
        };
        let history_json = serde_json::to_string(&record.history)
            .map_err(|error| format!("encode composer history: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO composer_sessions(
                   session_id, text, cursor, selection_start, selection_end, history_json, updated_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(session_id) DO UPDATE SET
                   text=excluded.text,
                   cursor=excluded.cursor,
                   selection_start=excluded.selection_start,
                   selection_end=excluded.selection_end,
                   history_json=excluded.history_json,
                   updated_ms=excluded.updated_ms",
                params![
                    session_id,
                    &record.text,
                    usize_to_i64(record.cursor),
                    usize_to_i64(record.selection_start),
                    usize_to_i64(record.selection_end),
                    history_json,
                    u64_to_i64(now_ms()),
                ],
            )
            .map(|_| ())
            .map_err(|error| format!("save composer session {}: {error}", record.target))
    }

    pub(crate) fn delete_composer_session(&self, target: &str) -> Result<(), String> {
        let Some(session_id) = self.session_id_for_target(target, None, None)? else {
            return Ok(());
        };
        self.connection
            .execute(
                "DELETE FROM composer_sessions WHERE session_id=?1",
                [session_id],
            )
            .map(|_| ())
            .map_err(|error| format!("delete composer session {target}: {error}"))
    }
}
