use super::*;

impl StateStore {
    pub(crate) fn load_composer_sessions(&self) -> Result<Vec<ComposerRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT target, text, cursor, selection_start, selection_end, history_json
                   FROM composer_sessions",
            )
            .map_err(|error| format!("prepare composer sessions: {error}"))?;
        statement
            .query_map([], |row| {
                let history_json = row.get::<_, String>(5)?;
                let history = serde_json::from_str(&history_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(ComposerRecord {
                    target: row.get(0)?,
                    text: row.get(1)?,
                    cursor: row.get::<_, u64>(2)?.try_into().unwrap_or(usize::MAX),
                    selection_start: row.get::<_, u64>(3)?.try_into().unwrap_or(usize::MAX),
                    selection_end: row.get::<_, u64>(4)?.try_into().unwrap_or(usize::MAX),
                    history,
                })
            })
            .map_err(|error| format!("query composer sessions: {error}"))?
            .map(|row| row.map_err(|error| format!("decode composer session: {error}")))
            .collect()
    }

    pub(crate) fn save_composer_session(&self, record: &ComposerRecord) -> Result<(), String> {
        let history_json = serde_json::to_string(&record.history)
            .map_err(|error| format!("encode composer history: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO composer_sessions(
                   target, text, cursor, selection_start, selection_end, history_json, updated_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(target) DO UPDATE SET
                   text=excluded.text,
                   cursor=excluded.cursor,
                   selection_start=excluded.selection_start,
                   selection_end=excluded.selection_end,
                   history_json=excluded.history_json,
                   updated_ms=excluded.updated_ms",
                params![
                    &record.target,
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
        self.connection
            .execute("DELETE FROM composer_sessions WHERE target=?1", [target])
            .map(|_| ())
            .map_err(|error| format!("delete composer session {target}: {error}"))
    }
}
