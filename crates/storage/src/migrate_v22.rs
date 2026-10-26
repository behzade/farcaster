use super::*;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let has_fork_source = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions')
               WHERE name='forked_from_backend_id')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("inspect fork ancestry column: {error}"))?;
    if !has_fork_source {
        tx.execute_batch("ALTER TABLE sessions ADD COLUMN forked_from_backend_id TEXT;")
            .map_err(|error| format!("add fork ancestry: {error}"))?;
    }
    let candidates = tx
        .prepare(
            "WITH RECURSIVE family_archive(id,archived_at) AS (
                SELECT id,archived_at FROM sessions WHERE parent_id IS NULL
                UNION
                SELECT child.id,parent.archived_at FROM sessions child
                  JOIN family_archive parent ON child.parent_id=parent.id
             )
             SELECT s.id,s.harness,s.locator,
                    COALESCE(parent.backend_id,s.parent_backend_id),family_archive.archived_at
               FROM sessions s LEFT JOIN sessions parent ON parent.id=s.parent_id
               LEFT JOIN family_archive ON family_archive.id=s.id
              WHERE s.locator IS NOT NULL AND s.forked_from_backend_id IS NULL
                AND (s.parent_id IS NOT NULL OR s.parent_backend_id IS NOT NULL)
                AND NOT EXISTS(SELECT 1 FROM worker_families WHERE child_id=s.id)",
        )
        .map_err(|error| error.to_string())?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    for (id, harness, locator, parent, archived_at) in candidates {
        let Ok(harness) = harness.parse() else {
            continue;
        };
        let Some(source) = agents::session_fork_source(harness, Path::new(&locator)) else {
            continue;
        };
        if parent.as_deref() != Some(source.as_str()) {
            continue;
        }
        tx.execute(
            // Keep the old family's archive visibility when making the fork a root.
            "UPDATE sessions SET forked_from_backend_id=?2,
               archived_at=COALESCE(archived_at,?3),
               parent_id=NULL,parent_backend_id=NULL WHERE id=?1",
            params![id, source, archived_at],
        )
        .map_err(|error| format!("separate saved fork ancestry: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "migrate_v22_tests.rs"]
mod tests;
