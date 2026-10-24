use super::*;

#[test]
fn repairs_verified_forks_and_keeps_workers_and_unknown_links()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let summaries = ["source", "fork", "worker", "marked-worker", "missing"].map(|id| {
        let path = temp.path().join(format!("{id}.jsonl"));
        if id != "missing" {
            std::fs::write(
                &path,
                serde_json::json!({
                    "type": "session", "id": id, "cwd": temp.path(),
                    "parentSession": (id != "source").then_some("source"),
                    "farcasterParentSession": (id == "marked-worker").then_some("source"),
                })
                .to_string(),
            )
            .expect("write session header");
        }
        SessionSummary::from_cached_for_harness(
            id.into(),
            Backend::Pi,
            path,
            temp.path().into(),
            id.into(),
            String::new(),
            String::new(),
            (id != "source").then(|| "source".into()),
            SystemTime::now(),
            0,
            Default::default(),
            false,
            false,
            String::new(),
        )
    });
    store.index_sessions(&summaries, false)?;
    store.connection.execute_batch(
        "INSERT INTO worker_families(child_id) SELECT id FROM sessions WHERE backend_id='worker';
         ALTER TABLE sessions DROP COLUMN forked_from_backend_id;
         UPDATE meta SET value='21' WHERE key='schema_version';",
    )?;
    drop(store);
    for _ in 0..2 {
        let store = StateStore::open_at(&database)?;
        let sessions = store.cached_sessions("")?;
        let fork = sessions.iter().find(|s| s.id == "fork").expect("fork");
        assert_eq!(fork.forked_from.as_deref(), Some("source"));
        assert!(fork.parent_session.is_none());
        assert!(fork.parent_app_session_id.is_none());
        for id in ["worker", "marked-worker", "missing"] {
            let worker = sessions.iter().find(|s| s.id == id).expect(id);
            assert_eq!(worker.parent_session.as_deref(), Some("source"));
            assert!(worker.parent_app_session_id.is_some());
        }
    }
    Ok(())
}
