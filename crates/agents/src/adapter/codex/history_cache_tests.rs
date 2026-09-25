use super::*;
use serde_json::json;
use std::{cell::Cell, io::Cursor};

fn fixture() -> (tempfile::TempDir, Connection, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let rollout = home.path().join("rollout.jsonl");
    std::fs::write(&rollout, "initial rollout").unwrap();
    let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
        CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, history_mode TEXT,
          model_provider TEXT, model TEXT, reasoning_effort TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('thread', ?1, 'legacy', 'openai', 'model-a', 'high')",
        [rollout.to_str().unwrap()],
    )
    .unwrap();
    (home, db, rollout)
}

fn scope() -> Scope {
    Scope::new(&Command::new("codex"), None, None)
}

#[test]
fn hit_skips_thread_read_and_preserves_full_history() {
    let (home, _db, _) = fixture();
    let response = json!({"id": 1, "result": {"thread": {"turns": [{"items": [
        {"type":"userMessage", "clientId":"farcaster-normal-codex-cli-prompt", "content":[{"type":"text", "text":"hello"}]},
        {"type":"agentMessage", "text":"answer"}
    ]}]}}});
    let mut requests = Vec::new();
    let mut connection =
        super::super::CodexConnection::new(Cursor::new(format!("{response}\n")), &mut requests);
    for _ in 0..2 {
        let history = load(home.path(), "thread", scope(), || {
            super::super::read_history(&mut connection, home.path(), "thread")
        })
        .unwrap();
        assert_eq!(history.messages.len(), 2);
        assert_eq!(history.messages[1]["content"][0]["text"], "answer");
        assert_eq!(history.model, Some(("openai".into(), "model-a".into())));
        assert_eq!(history.thinking_level.as_deref(), Some("high"));
        let delivery = history.prompt_deliveries.unwrap();
        assert_eq!(delivery.delivered, ["codex-cli-prompt"]);
        assert!(delivery.pending.is_empty());
        assert!(!delivery.absence_is_not_delivered);
    }
    drop(connection);
    let requests = String::from_utf8(requests).unwrap();
    assert_eq!(requests.lines().count(), 1);
    let request: serde_json::Value = serde_json::from_str(requests.trim()).unwrap();
    assert_eq!(request["method"], "thread/read");
    assert_eq!(request["params"]["includeTurns"], true);
}

fn counted_load(home: &Path, scope: Scope, calls: &Cell<usize>) {
    load(home, "thread", scope, || {
        calls.set(calls.get() + 1);
        Ok(DiscoveredHistory {
            messages: vec![],
            model: None,
            thinking_level: None,
            prompt_deliveries: None,
        })
    })
    .unwrap();
}

#[test]
fn rollout_identity_and_history_wal_writes_invalidate() {
    let (home, db, rollout) = fixture();
    let calls = Cell::new(0);
    counted_load(home.path(), scope(), &calls);
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 1);
    db.execute_batch("CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('startup maintenance');").unwrap();
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 1);
    std::fs::write(&rollout, "extended rollout contents").unwrap();
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 2);
    let main_before = FileStamp::read(&home.path().join("state_5.sqlite")).unwrap();
    db.execute(
        "UPDATE threads SET model='model-b', reasoning_effort='low'",
        [],
    )
    .unwrap();
    assert!(FileStamp::read(&home.path().join("state_5.sqlite")).as_ref() == Some(&main_before));
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 3);

    let history = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
    history
        .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items (text TEXT);")
        .unwrap();
    db.execute("UPDATE threads SET history_mode='paginated'", [])
        .unwrap();
    counted_load(home.path(), scope(), &calls);
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 4);
    history
        .execute("INSERT INTO items VALUES ('new output')", [])
        .unwrap();
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 5);
}

#[test]
fn missing_or_unknown_sources_bypass_and_changed_reads_are_not_cached() {
    let (home, db, rollout) = fixture();
    let calls = Cell::new(0);
    counted_load(home.path(), scope(), &calls);
    std::fs::remove_file(&rollout).unwrap();
    counted_load(home.path(), scope(), &calls);
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 3);
    std::fs::write(&rollout, "restored").unwrap();
    for mode in ["unknown", "paginated"] {
        db.execute("UPDATE threads SET history_mode=?1", [mode])
            .unwrap();
        assert!(revision(home.path(), "thread").is_none());
    }
    db.execute("UPDATE threads SET history_mode='legacy'", [])
        .unwrap();
    load(home.path(), "thread", scope(), || {
        std::fs::write(&rollout, "changed while reading").unwrap();
        Ok(DiscoveredHistory {
            messages: vec![],
            model: None,
            thinking_level: None,
            prompt_deliveries: None,
        })
    })
    .unwrap();
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 4);
    assert!(revision(Path::new("relative"), "thread").is_none());
    assert!(revision(home.path(), "missing-thread").is_none());
}

#[test]
fn launch_project_profile_and_home_do_not_share_entries() {
    let (home, _db, _) = fixture();
    let (other_home, _other_db, _) = fixture();
    let calls = Cell::new(0);
    counted_load(home.path(), scope(), &calls);
    counted_load(other_home.path(), scope(), &calls);
    let mut project = scope();
    project.project = Some(home.path().join("project"));
    counted_load(home.path(), project, &calls);
    let mut profile = scope();
    profile.profile = Some("other-profile".into());
    counted_load(home.path(), profile, &calls);
    let mut command = Command::new("other-codex");
    command.arg("--config").env("CODEX_HOME", home.path());
    counted_load(home.path(), Scope::new(&command, None, None), &calls);
    assert_eq!(calls.get(), 5);
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 5);
}

#[cfg(unix)]
#[test]
fn symlinked_databases_track_target_wal() {
    let (native, db, _) = fixture();
    let home = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(
        native.path().join("state_5.sqlite"),
        home.path().join("state_5.sqlite"),
    )
    .unwrap();
    let calls = Cell::new(0);
    counted_load(home.path(), scope(), &calls);
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 1);
    db.execute("UPDATE threads SET model='changed-through-real-wal'", [])
        .unwrap();
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 2);
    let history = Connection::open(native.path().join("thread_history_1.sqlite")).unwrap();
    history
        .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items (text TEXT);")
        .unwrap();
    std::os::unix::fs::symlink(
        native.path().join("thread_history_1.sqlite"),
        home.path().join("thread_history_1.sqlite"),
    )
    .unwrap();
    db.execute("UPDATE threads SET history_mode='paginated'", [])
        .unwrap();
    counted_load(home.path(), scope(), &calls);
    history
        .execute("INSERT INTO items VALUES ('new item')", [])
        .unwrap();
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 4);
}

#[test]
fn live_threads_and_relocated_or_unknown_config_bypass_cache() {
    let home = tempfile::tempdir().unwrap();
    for (loaded, config, expected) in [
        (json!({"data": []}), json!({"config": {}}), true),
        (
            json!({"data": ["live-thread"]}),
            json!({"config": {}}),
            false,
        ),
        (
            json!({"data": []}),
            json!({"config": {"sqlite_home": "/missing-relocated-db"}}),
            false,
        ),
        (json!({"data": []}), json!({}), false),
    ] {
        let input = format!(
            "{}\n{}\n",
            json!({"id": 1, "result": loaded}),
            json!({"id": 2, "result": config})
        );
        let mut requests = Vec::new();
        let mut connection = super::super::CodexConnection::new(Cursor::new(input), &mut requests);
        assert_eq!(
            super::super::can_cache_persisted_history(&mut connection, home.path()),
            expected
        );
    }
    let mut command = Command::new("codex");
    command.env("CODEX_SQLITE_HOME", home.path());
    assert!(Scope::new(&command, None, None).has_sqlite_home_override());
}
