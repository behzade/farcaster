use super::*;

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let has_routing = tx
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM pragma_table_info('worker_families') WHERE name='routing_json'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("inspect worker family columns: {error}"))?;
    if !has_routing {
        tx.execute_batch("ALTER TABLE worker_families ADD COLUMN routing_json TEXT;")
            .map_err(|error| format!("add durable worker routing: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "migrate_v18_tests.rs"]
mod tests;
