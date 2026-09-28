use super::*;
use crate::sessions::AppSessionId;

const MIGRATION_KEY: &str = "workgraph_application_session_keys";

struct IndexedIdentity {
    key: String,
    native_id: Option<String>,
    locator: Option<PathBuf>,
    project: PathBuf,
}

pub(super) fn migrate(connection: &mut Connection) -> Result<(), String> {
    if migrated(connection)? {
        return Ok(());
    }
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("start workgraph identity migration: {error}"))?;
    migrate_in_transaction(&tx)?;
    tx.commit()
        .map_err(|error| format!("commit workgraph identity migration: {error}"))
}

fn migrated(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)",
            [MIGRATION_KEY],
            |row| row.get(0),
        )
        .map_err(|error| format!("read workgraph identity migration: {error}"))
}

fn migrate_in_transaction(tx: &Transaction<'_>) -> Result<(), String> {
    if migrated(tx)? {
        return Ok(());
    }
    let identities = tx.prepare(
        "SELECT s.id,COALESCE(s.backend_id,s.locator),s.locator,p.path FROM sessions s JOIN projects p ON p.id=s.project_id",
    ).map_err(|error| error.to_string())?
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            Ok((id, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?))
        }).map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|error| error.to_string())?
        .into_iter().map(|(id, native_id, locator, project)| {
            Ok(IndexedIdentity {
                key: AppSessionId::try_from(id)?.to_key(),
                native_id,
                locator: locator.map(|path| crate::sessions::normalize_session_path(Path::new(&path))),
                project: crate::sessions::normalize_session_path(Path::new(&project)),
            })
        }).collect::<Result<Vec<_>, String>>()?;
    workgraph::remap_session_keys(tx, |project, native_id, locator| {
        let project = crate::sessions::normalize_session_path(Path::new(project));
        let locator = locator.map(|path| crate::sessions::normalize_session_path(Path::new(path)));
        let mut matches = identities.iter().filter(|identity| {
            identity.project == project
                && identity.native_id.as_deref() == Some(native_id)
                && locator
                    .as_ref()
                    .is_none_or(|path| identity.locator.as_ref() == Some(path))
        });
        match (matches.next(), matches.next()) {
            (Some(identity), None) => Some(identity.key.clone()),
            // Legacy native IDs are opaque too. Quarantine a reserved-key
            // lookalike rather than accidentally granting it application ownership.
            _ if AppSessionId::from_key(native_id).is_some() => {
                Some(format!("legacy-session:{native_id}"))
            }
            _ => None,
        }
    })
    .map_err(|error| format!("migrate workgraph session keys: {error}"))?;
    tx.execute(
        "INSERT INTO meta(key,value) VALUES(?1,'1')",
        [MIGRATION_KEY],
    )
    .map_err(|error| format!("record workgraph identity migration: {error}"))?;
    Ok(())
}

/// Called before the removed session row is deleted, inside the same transaction.
pub(super) fn merge(tx: &Transaction<'_>, keep: i64, other: i64) -> Result<(), String> {
    migrate_in_transaction(tx)?;
    let keep = AppSessionId::try_from(keep)?.to_key();
    let other = AppSessionId::try_from(other)?.to_key();
    workgraph::remap_session_keys(tx, |_, key, _| (key == other).then(|| keep.clone()))
        .map_err(|error| format!("merge workgraph session keys: {error}"))
}

#[cfg(test)]
#[path = "workgraph_identity_tests.rs"]
mod tests;
