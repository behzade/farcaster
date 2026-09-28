use std::path::{Component, Path};

use rusqlite::{TransactionBehavior, params};

pub fn relocate_snapshot_session_locators(
    database: &Path,
    source_data_directory: &Path,
    destination_data_directory: &Path,
) -> Result<usize, String> {
    let source_root = std::path::absolute(source_data_directory)
        .map_err(|error| format!("resolve source app data directory: {error}"))?
        .join("session-locators");
    let destination_root = std::path::absolute(destination_data_directory)
        .map_err(|error| format!("resolve destination app data directory: {error}"))?
        .join("session-locators");
    let normalized_source_root = crate::sessions::normalize_session_path(&source_root);
    let destination_root = crate::sessions::normalize_session_path(&destination_root);
    if !std::fs::metadata(database)
        .map_err(|error| format!("inspect snapshot for relocation: {error}"))?
        .is_file()
    {
        return Err("snapshot for relocation must be a regular database file".into());
    }
    let mut store = crate::StateStore::open_at(database)?;
    let transaction = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("start snapshot locator relocation: {error}"))?;
    let locators = transaction
        .prepare("SELECT id, locator FROM sessions WHERE locator IS NOT NULL ORDER BY id")
        .map_err(|error| format!("read snapshot locators: {error}"))?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query snapshot locators: {error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("decode snapshot locators: {error}"))?;
    let mut changed = 0;
    for (id, locator) in locators {
        let path = Path::new(&locator);
        let Ok(suffix) = path
            .strip_prefix(&normalized_source_root)
            .or_else(|_| path.strip_prefix(&source_root))
        else {
            continue;
        };
        if suffix.as_os_str().is_empty()
            || !suffix
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            continue;
        }
        let relocated = destination_root.join(suffix);
        let relocated = relocated
            .to_str()
            .ok_or_else(|| "relocated session locator must have a UTF-8 path".to_owned())?;
        if relocated == locator {
            continue;
        }
        changed += transaction
            .execute(
                "UPDATE sessions SET locator=?2 WHERE id=?1",
                params![id, relocated],
            )
            .map_err(|error| format!("relocate snapshot session {id}: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("commit snapshot locator relocation: {error}"))?;
    Ok(changed)
}

#[cfg(test)]
#[path = "relocation_tests.rs"]
mod tests;
