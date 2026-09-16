use super::*;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let has_access_mode = tx
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM pragma_table_info('sessions') WHERE name='access_mode'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("inspect session access-mode column: {error}"))?;
    if !has_access_mode {
        tx.execute_batch(
            "ALTER TABLE sessions ADD COLUMN access_mode TEXT
               CHECK (access_mode IN ('sandboxed', 'auto', 'full'));",
        )
        .map_err(|error| format!("add durable session access mode: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "migrate_v19_tests.rs"]
mod tests;
