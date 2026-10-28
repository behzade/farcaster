use super::*;

#[test]
fn migrates_verified_codex_forks_from_their_profile_home() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let home = temp.path().join("codex-home");
    std::fs::create_dir(&home)?;
    let native = Connection::open(home.join("state_5.sqlite"))?;
    native
        .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL);")?;
    let profile = "11111111-1111-4111-8111-111111111111";
    let mut store = StateStore::open_at(&database)?;
    store.save_harness_profiles(&[agents::HarnessProfile {
        id: profile.into(),
        name: "test".into(),
        backend: Backend::Codex,
        executable: "codex".into(),
        data_directory: Some(home.clone()),
    }])?;
    let ids = [
        "source",
        "fork",
        "nested",
        "worker",
        "native-worker",
        "unknown",
        "missing",
        "mismatch",
    ];
    let mut summaries = Vec::new();
    for id in ids {
        let parent = (id != "source").then_some(if id == "nested" { "fork" } else { "source" });
        let rollout = home.join(format!("{id}.jsonl"));
        if id != "missing" {
            std::fs::write(&rollout, serde_json::json!({
                "type": "session_meta", "payload": {
                    "id": id, "forked_from_id": if id == "mismatch" { Some("other") } else { parent },
                    "source": match id {
                        "native-worker" => serde_json::json!({"subAgent": {"thread_spawn": {"parent_thread_id": "source"}}}),
                        "unknown" => serde_json::Value::Null,
                        _ => serde_json::json!("vscode"),
                    },
                },
            }).to_string())?;
        }
        native.execute(
            "INSERT INTO threads VALUES(?1,?2)",
            params![id, rollout.to_string_lossy()],
        )?;
        summaries.push(SessionSummary::from_cached_for_harness(
            id.into(),
            Backend::Codex,
            temp.path()
                .join(format!("profiles/{profile}/codex-cli/{id}")),
            temp.path().into(),
            id.into(),
            String::new(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::now(),
            0,
            Default::default(),
            false,
            false,
            String::new(),
        ));
    }
    store.index_sessions(&summaries, false)?;
    store.connection.execute_batch(
        "UPDATE sessions SET archived_at=123 WHERE backend_id='source';
         INSERT INTO worker_families(child_id) SELECT id FROM sessions WHERE backend_id='worker';
         ALTER TABLE sessions DROP COLUMN forked_from_backend_id;
         UPDATE meta SET value='21' WHERE key='schema_version';",
    )?;
    drop(store);
    for _ in 0..2 {
        let store = StateStore::open_at(&database)?;
        let cached = store.cached_sessions("")?;
        assert_eq!(cached.len(), ids.len());
        for id in ["fork", "nested"] {
            let fork = cached.iter().find(|s| s.id == id).expect(id);
            assert!(fork.parent_app_session_id.is_none());
            assert!(fork.parent_session.is_none());
            assert!(fork.archived);
            assert_eq!(
                fork.forked_from.as_deref(),
                Some(if id == "nested" { "fork" } else { "source" })
            );
        }
        for id in ["worker", "native-worker", "unknown", "missing", "mismatch"] {
            let child = cached.iter().find(|s| s.id == id).expect(id);
            assert!(child.parent_app_session_id.is_some(), "{id}");
            assert!(child.forked_from.is_none(), "{id}");
        }
    }
    Ok(())
}

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

#[test]
fn separating_legacy_forks_preserves_their_family_archive_visibility()
-> Result<(), Box<dyn std::error::Error>> {
    for archived in [false, true] {
        let temp = tempfile::tempdir()?;
        let database = temp.path().join("state.sqlite3");
        let mut store = StateStore::open_at(&database)?;
        let summaries = [
            "source",
            "nested-fork",
            "fork",
            "already-archived",
            "worker",
        ]
        .map(|id| {
            let parent = match id {
                "source" => None,
                "nested-fork" => Some("fork"),
                _ => Some("source"),
            };
            let path = temp.path().join(format!("{id}.jsonl"));
            std::fs::write(
                &path,
                serde_json::json!({
                    "type": "session", "id": id, "cwd": temp.path(),
                    "parentSession": parent,
                })
                .to_string(),
            )
            .expect("write session header");
            SessionSummary::from_cached_for_harness(
                id.into(),
                Backend::Pi,
                path,
                temp.path().into(),
                id.into(),
                String::new(),
                String::new(),
                parent.map(str::to_owned),
                SystemTime::now(),
                0,
                Default::default(),
                false,
                false,
                String::new(),
            )
        });
        store.index_sessions(&summaries, false)?;
        store.connection.execute(
            "UPDATE sessions SET archived_at=?1 WHERE backend_id='source'",
            [archived.then_some(123_i64)],
        )?;
        store.connection.execute_batch(
            "UPDATE sessions SET archived_at=456 WHERE backend_id='already-archived';
             INSERT INTO worker_families(child_id) SELECT id FROM sessions WHERE backend_id='worker';
             ALTER TABLE sessions DROP COLUMN forked_from_backend_id;
             UPDATE meta SET value='21' WHERE key='schema_version';",
        )?;
        drop(store);
        for _ in 0..2 {
            let store = StateStore::open_at(&database)?;
            let sessions = store.cached_sessions("")?;
            assert_eq!(sessions.len(), 5, "migration must preserve every session");
            for id in ["fork", "nested-fork"] {
                let fork = sessions.iter().find(|s| s.id == id).expect(id);
                assert_eq!(
                    fork.archived, archived,
                    "{id} must keep its family visibility"
                );
                assert!(fork.parent_session.is_none());
                assert!(fork.parent_app_session_id.is_none());
                assert_eq!(
                    fork.forked_from.as_deref(),
                    Some(if id == "fork" { "source" } else { "fork" })
                );
            }
            let saved_archive: i64 = store.connection.query_row(
                "SELECT archived_at FROM sessions WHERE backend_id='already-archived'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(saved_archive, 456, "preserve a fork's own archive state");
            let worker = sessions.iter().find(|s| s.id == "worker").expect("worker");
            assert!(worker.parent_app_session_id.is_some());
            assert_eq!(worker.parent_session.as_deref(), Some("source"));
            assert!(worker.forked_from.is_none());
        }
    }
    Ok(())
}
