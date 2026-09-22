use super::*;

#[test]
fn upgrades_v18_sessions_with_an_empty_access_mode() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&path)?;
    store.connection.execute_batch(
        "ALTER TABLE sessions DROP COLUMN access_mode;
         UPDATE meta SET value='18' WHERE key='schema_version';
         INSERT INTO projects(id,path,added_ms) VALUES(1,'/project',0);
         INSERT INTO sessions(id,project_id,harness,locator,modified_ms,created_ms)
         VALUES(1,1,'pi','/session',0,0);",
    )?;
    drop(store);

    let store = StateStore::open_at(&path)?;
    let row: (Option<String>, i64) = store.connection.query_row(
        "SELECT access_mode,
                (SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version')
           FROM sessions WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(row, (None, 20));
    Ok(())
}
