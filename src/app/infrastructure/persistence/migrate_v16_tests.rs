use super::*;
use crate::agents::Backend;

#[test]
fn upgrades_saved_opencode_sessions_and_settings_without_changing_user_content()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&path)?;
    store.connection.execute_batch(
        r#"UPDATE meta SET value='15' WHERE key='schema_version';
         INSERT INTO projects(id, path, added_ms) VALUES(1, '/project/opencode2', 0);
         INSERT INTO sessions(id, project_id, harness, locator, backend_id, parent_id, title, modified_ms, created_ms)
         VALUES(1, 1, 'opencode2', '/locators/opencode2/ses_root', 'ses_root', NULL, 'opencode2', 0, 0),
               (2, 1, 'opencode2', '/locators/opencode2/ses_child', 'ses_child', 1, '', 0, 0),
               (3, 1, 'opencode2', NULL, NULL, NULL, '', 0, 0),
               (4, 1, 'pi', '/project/opencode2/file.jsonl', 'pi-id', NULL, '', 0, 0);
         INSERT INTO meta(key, value) VALUES('preferred_harness', 'opencode2');
         INSERT INTO session_events(session_id, seq, t, schema_version, body)
         VALUES(1, 1, 0, 1, '{"text":"opencode2","submissionId":"opencode2-123"}');
         INSERT INTO worker_families(child_id, execution_json)
         VALUES(2, '{"harness":"opencode2","model":"opencode2"}');
         UPDATE ui_state SET
           worker_tasks_json='{"profiles":[{"name":"opencode2","models":[{"harness":"opencode2","model":"astra"}]}]}',
           configuration_catalogs_json='[{"harness":"opencode2"}]',
           session_control_defaults_json='[{"harness":"opencode2"}]';"#
    )?;
    drop(store);

    let store = StateStore::open_at(&path)?;
    let mut statement = store
        .connection
        .prepare("SELECT harness, locator FROM sessions ORDER BY id")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        rows,
        vec![
            (
                "opencode".into(),
                Some("/locators/opencode/ses_root".into())
            ),
            (
                "opencode".into(),
                Some("/locators/opencode/ses_child".into())
            ),
            ("opencode".into(), None),
            ("pi".into(), Some("/project/opencode2/file.jsonl".into())),
        ]
    );
    assert_eq!(
        store
            .load_preferred_harness(Path::new("/project/opencode2"))?
            .map(Backend::as_str),
        Some("opencode")
    );
    let child: (String, i64) = store.connection.query_row(
        "SELECT backend_id, parent_id FROM sessions WHERE id=2",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    assert_eq!(child, ("ses_child".into(), 1));
    for column in [
        "worker_tasks_json",
        "configuration_catalogs_json",
        "session_control_defaults_json",
    ] {
        let value: String =
            store
                .connection
                .query_row(&format!("SELECT {column} FROM ui_state"), [], |r| r.get(0))?;
        assert!(value.contains("\"harness\":\"opencode\""));
        assert!(!value.contains("\"harness\":\"opencode2\""));
        if column == "worker_tasks_json" {
            assert!(value.contains("\"name\":\"opencode2\""));
        }
    }
    let execution: String =
        store
            .connection
            .query_row("SELECT execution_json FROM worker_families", [], |r| {
                r.get(0)
            })?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&execution)?,
        serde_json::json!({"harness":"opencode", "model":"opencode2"})
    );
    let body: String = store
        .connection
        .query_row("SELECT body FROM session_events", [], |r| r.get(0))?;
    assert_eq!(
        body,
        r#"{"text":"opencode2","submissionId":"opencode2-123"}"#
    );
    let title: String =
        store
            .connection
            .query_row("SELECT title FROM sessions WHERE id=1", [], |r| r.get(0))?;
    assert_eq!(title, "opencode2");
    drop(statement);
    drop(store);
    let store = StateStore::open_at(&path)?;
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))?,
        4
    );
    Ok(())
}
