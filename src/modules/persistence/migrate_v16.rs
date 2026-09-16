use super::*;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let mut statement = tx
        .prepare("SELECT id, locator FROM sessions WHERE harness='opencode2'")
        .map_err(|error| format!("read old OpenCode sessions: {error}"))?;
    let sessions = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| format!("query old OpenCode sessions: {error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("decode old OpenCode sessions: {error}"))?;
    for (id, locator) in sessions {
        let locator = locator.map(|locator| {
            let path = Path::new(&locator);
            match (path.parent(), path.file_name()) {
                (Some(parent), Some(name))
                    if parent.file_name().is_some_and(|name| name == "opencode2") =>
                {
                    parent
                        .with_file_name("opencode")
                        .join(name)
                        .to_string_lossy()
                        .into_owned()
                }
                _ => locator,
            }
        });
        tx.execute(
            "UPDATE sessions SET harness='opencode', locator=?1 WHERE id=?2",
            params![locator, id],
        )
        .map_err(|error| format!("rename OpenCode session {id}: {error}"))?;
    }
    tx.execute(
        "UPDATE meta SET value='opencode' WHERE key='preferred_harness' AND value='opencode2'",
        [],
    )
    .map_err(|error| format!("rename preferred OpenCode backend: {error}"))?;

    for (table, key, column) in [
        ("ui_state", "id", "worker_tasks_json"),
        ("ui_state", "id", "configuration_catalogs_json"),
        ("ui_state", "id", "session_control_defaults_json"),
        ("worker_families", "child_id", "execution_json"),
    ] {
        let mut statement = tx
            .prepare(&format!(
                "SELECT {key}, {column} FROM {table} WHERE {column} IS NOT NULL"
            ))
            .map_err(|error| format!("read {column} for OpenCode migration: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query {column}: {error}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("decode {column}: {error}"))?;
        for (id, json) in rows {
            let mut value: serde_json::Value = serde_json::from_str(&json)
                .map_err(|error| format!("decode {column} JSON: {error}"))?;
            if rename_harnesses(&mut value) {
                tx.execute(
                    &format!("UPDATE {table} SET {column}=?1 WHERE {key}=?2"),
                    params![value.to_string(), id],
                )
                .map_err(|error| format!("rename OpenCode in {column}: {error}"))?;
            }
        }
    }
    Ok(())
}

fn rename_harnesses(value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == "harness" && value.as_str() == Some("opencode2") {
                    *value = "opencode".into();
                    changed = true;
                } else {
                    changed |= rename_harnesses(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                changed |= rename_harnesses(value);
            }
        }
        _ => {}
    }
    changed
}

#[cfg(test)]
#[path = "migrate_v16_tests.rs"]
mod tests;
