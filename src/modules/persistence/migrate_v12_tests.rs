use super::*;

#[test]
fn worker_families_keep_legacy_backend_names_until_the_backend_migration()
-> Result<(), Box<dyn std::error::Error>> {
    let mut connection = Connection::open_in_memory()?;
    connection.execute_batch(
        "CREATE TABLE meta(key TEXT, value TEXT);
         CREATE TABLE projects(id INTEGER PRIMARY KEY, path TEXT);
         CREATE TABLE sessions(id INTEGER PRIMARY KEY, project_id INTEGER, harness TEXT,
           locator TEXT, backend_id TEXT, parent_id INTEGER);
         CREATE TABLE worker_families(child_id INTEGER PRIMARY KEY, execution_json TEXT);
         INSERT INTO projects VALUES(1, '/project');
         INSERT INTO sessions VALUES(1, 1, 'opencode2', '/old/parent', 'parent', NULL);
         INSERT INTO sessions VALUES(2, 1, 'opencode2', '/old/child', 'child', NULL);",
    )?;
    connection.execute(
        "INSERT INTO meta VALUES('worker_family:child', ?1)",
        [serde_json::json!({
            "project": "/project",
            "child_backend": "opencode2",
            "child_session": "child",
            "parent_backend": "opencode2",
            "parent_session": "parent",
            "execution": {"harness": "opencode2", "provider": "openai", "model": "astra"}
        })
        .to_string()],
    )?;
    let transaction = connection.transaction()?;
    copy_worker_families(&transaction)?;
    transaction.commit()?;
    let parent: i64 =
        connection.query_row("SELECT parent_id FROM sessions WHERE id=2", [], |row| {
            row.get(0)
        })?;
    assert_eq!(parent, 1);
    let execution: String = connection.query_row(
        "SELECT execution_json FROM worker_families WHERE child_id=2",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&execution)?["harness"],
        "opencode2"
    );
    let remaining: i64 = connection.query_row("SELECT COUNT(*) FROM meta", [], |row| row.get(0))?;
    assert_eq!(remaining, 0);
    Ok(())
}
