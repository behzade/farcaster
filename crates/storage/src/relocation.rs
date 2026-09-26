//! Rebase synthetic session locators in a private database snapshot.

use std::path::{Component, Path};

use rusqlite::{TransactionBehavior, params};

/// Rebase synthetic session paths below the source app's `session-locators`.
///
/// Call this on the private destination database after `snapshot_database`.
/// It first opens the snapshot as a `StateStore`, running the normal migrations
/// and identity repair only on the private copy. Relocation then changes only
/// `sessions.locator`; IDs and native backend paths stay unchanged. No locator
/// files are copied. Returns the number of relocated sessions.
///
/// A relocation conflict rolls back the path changes, but not the preceding
/// migration. The caller must discard the private copy if this returns an error.
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
    // Do not let StateStore create a fresh database when a snapshot is missing.
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
        let Ok(suffix) = Path::new(&locator).strip_prefix(&source_root) else {
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
