use super::*;

#[test]
fn upgrades_v19_outbox_states_without_losing_rows() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&path)?;
    store.connection.execute_batch(
        "INSERT INTO projects(id,path,added_ms) VALUES(1,'/project',0);
         INSERT INTO sessions(id,project_id,harness,locator,modified_ms,created_ms)
         VALUES(1,1,'pi','/session',0,0);
         ALTER TABLE outbox RENAME TO outbox_v19;
         CREATE TABLE outbox (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           submission_event_seq INTEGER,
           mode TEXT NOT NULL,
           message TEXT NOT NULL,
           display_message TEXT,
           invocation TEXT,
           images_json TEXT NOT NULL DEFAULT '[]',
           provider TEXT,
           model TEXT,
           effort TEXT,
           service_tier TEXT,
           state TEXT NOT NULL DEFAULT 'queued'
             CHECK (state IN ('queued', 'sending', 'failed', 'unknown', 'pending', 'acked', 'cancelled')),
           error TEXT,
           created_ms INTEGER NOT NULL
         );
         INSERT INTO outbox(
           session_id, mode, message, state, created_ms
         ) VALUES
           (1, 'normal', 'queued', 'queued', 1),
           (1, 'follow_up', 'sending', 'sending', 2),
           (1, 'normal', 'failed', 'failed', 3),
           (1, 'follow_up', 'unknown', 'unknown', 4),
           (1, 'normal', 'pending', 'pending', 5),
           (1, 'follow_up', 'acked', 'acked', 6),
           (1, 'normal', 'cancelled', 'cancelled', 7);
         DROP TABLE outbox_v19;
         UPDATE meta SET value='19' WHERE key='schema_version';",
    )?;
    drop(store);

    let store = StateStore::open_at(&path)?;
    let states = store
        .connection
        .prepare("SELECT message, state FROM outbox ORDER BY id")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        states,
        vec![
            ("queued".into(), "pending".into()),
            ("sending".into(), "pending".into()),
            ("failed".into(), "pending".into()),
            ("unknown".into(), "pending".into()),
            ("pending".into(), "pending".into()),
            ("acked".into(), "acked".into()),
            ("cancelled".into(), "cancelled".into()),
        ]
    );
    assert_eq!(
        store.connection.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        20
    );
    let id = store.enqueue_prompt(
        "session:/session",
        crate::agents::Backend::Pi,
        std::path::Path::new("/project"),
        Some(std::path::Path::new("/session")),
        crate::protocol::PromptMode::Normal,
        "after migration",
        &[],
    )?;
    assert_eq!(
        store
            .connection
            .query_row("SELECT state FROM outbox WHERE id=?1", [id], |row| {
                row.get::<_, String>(0)
            })?,
        "pending"
    );
    Ok(())
}
