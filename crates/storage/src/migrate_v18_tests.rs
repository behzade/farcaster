use super::*;

#[test]
fn upgrades_v17_worker_families_without_changing_existing_execution()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&path)?;
    store.connection.execute_batch(
        "ALTER TABLE worker_families DROP COLUMN routing_json;
         UPDATE meta SET value='17' WHERE key='schema_version';
         INSERT INTO projects(id,path,added_ms) VALUES(1,'/project',0);
         INSERT INTO sessions(id,project_id,harness,backend_id,parent_id,modified_ms,created_ms)
         VALUES(1,1,'pi','parent',NULL,0,0),
               (2,1,'codex-cli','child',1,0,0);
         INSERT INTO worker_families(child_id,execution_json)
         VALUES(2,'{\"harness\":\"codex-cli\",\"provider\":\"openai\",\"model\":\"gpt\",\"effort\":null}');",
    )?;
    drop(store);

    let store = StateStore::open_at(&path)?;
    let row: (String, Option<String>) = store.connection.query_row(
        "SELECT execution_json,routing_json FROM worker_families WHERE child_id=2",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert!(row.0.contains("codex-cli"));
    assert_eq!(row.1, None);
    assert_eq!(
        store.connection.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        19
    );
    Ok(())
}
