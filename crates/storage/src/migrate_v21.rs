use rusqlite::Transaction;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let has_profile = tx
        .prepare("PRAGMA table_info(sessions)")
        .map_err(|error| error.to_string())?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|name| name == "profile_id");
    if !has_profile {
        tx.execute_batch("ALTER TABLE sessions ADD COLUMN profile_id TEXT;")
            .map_err(|error| format!("add session harness profile: {error}"))?;
    }
    Ok(())
}
