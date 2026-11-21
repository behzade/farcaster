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
    let mut scope = Scope::new(&Command::new("codex"), None, None);
    scope
        .inherited_env
        .retain(|(key, _)| key != "CODEX_SQLITE_HOME");
    scope
}

fn rpc_results(results: impl IntoIterator<Item = serde_json::Value>) -> Cursor<String> {
    Cursor::new(
        results
            .into_iter()
            .enumerate()
            .map(|(index, result)| format!("{}\n", json!({"id":index + 1,"result":result})))
            .collect(),
    )
}

fn paginated_fixture() -> (tempfile::TempDir, Connection, PathBuf) {
    let (home, db, rollout) = fixture();
    db.execute("UPDATE threads SET history_mode='paginated'", [])
        .unwrap();
    let history = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
    history
        .execute_batch(
            "CREATE TABLE thread_turns (thread_id TEXT);
         INSERT INTO thread_turns VALUES ('thread');
         CREATE TABLE thread_history_projection_state (
             thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER, next_rollout_ordinal INTEGER
         );",
        )
        .unwrap();
    history
        .execute(
            "INSERT INTO thread_history_projection_state VALUES ('thread', ?1, 1)",
            [rollout.metadata().unwrap().len()],
        )
        .unwrap();
    (home, db, rollout)
}

fn turn(text: &str) -> serde_json::Value {
    json!({"id":text,"itemsView":"full","status":"completed","items":[
        {"type":"agentMessage","id":format!("answer-{text}"),"text":text}
    ]})
}

#[test]
fn hit_skips_preview_and_thread_read_and_preserves_full_history() {
    let (home, _db, _) = paginated_fixture();
    let mut earlier = turn("first answer");
    earlier["items"].as_array_mut().unwrap().insert(
        0,
        json!({
            "id":"command","type":"commandExecution","command":"pwd","status":"completed",
            "aggregatedOutput":"/project"
        }),
    );
    let recent = turn("last answer");
    let page = json!({"data":[recent, earlier], "nextCursor":"earlier"});
    let response = json!({"thread": {"turns": [{"items": [
        {"type":"userMessage", "clientId":"farcaster-normal-codex-cli-prompt", "content":[{"type":"text", "text":"hello"}]}
    ]}, earlier, recent]}});
    let mut requests = Vec::new();
    let mut connection = super::super::CodexConnection::new(
        rpc_results([
            json!({"data":[]}),
            json!({"config":{}}),
            page,
            response,
            json!({"data":[]}),
            json!({"config":{}}),
        ]),
        &mut requests,
    );
    let mut previews = Vec::new();
    for _ in 0..2 {
        let history = super::super::load_history_with_client(
            &mut connection,
            home.path(),
            "thread",
            scope(),
            Some(&mut |messages| previews.push(messages)),
        )
        .unwrap();
        assert_eq!(history.messages.len(), 5);
        assert_eq!(history.messages[1]["content"][0]["type"], "toolCall");
        assert_eq!(history.messages[2]["content"][0]["text"], "/project");
        assert_eq!(history.messages[3]["content"][0]["text"], "first answer");
        assert_eq!(history.messages[4]["content"][0]["text"], "last answer");
        assert_eq!(&previews[0].content[..], &history.messages[1..]);
        assert_eq!(history.model, Some(("openai".into(), "model-a".into())));
        assert_eq!(history.thinking_level.as_deref(), Some("high"));
        let delivery = history.prompt_deliveries.unwrap();
        assert_eq!(delivery.delivered, ["codex-cli-prompt"]);
        assert!(delivery.pending.is_empty());
        assert!(!delivery.absence_is_not_delivered);
    }
    drop(connection);
    assert_eq!(previews.len(), 1);
    let requests = String::from_utf8(requests).unwrap();
    let mut requests = requests
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap());
    assert_eq!(requests.next().unwrap()["method"], "thread/loaded/list");
    assert_eq!(requests.next().unwrap()["method"], "config/read");
    let preview = requests.next().unwrap();
    assert_eq!(preview["method"], "thread/turns/list");
    assert_eq!(
        preview["params"],
        json!({"threadId":"thread","limit":8,"sortDirection":"desc","itemsView":"full"})
    );
    let request = requests.next().unwrap();
    assert_eq!(request["method"], "thread/read");
    assert_eq!(request["params"]["includeTurns"], true);
    assert_eq!(requests.next().unwrap()["method"], "thread/loaded/list");
    assert_eq!(requests.next().unwrap()["method"], "config/read");
    assert!(requests.next().is_none());
}

fn counted_load(home: &Path, scope: Scope, calls: &Cell<usize>) {
    load(home, "thread", scope, || {
        calls.set(calls.get() + 1);
        Ok(DiscoveredHistory {
            messages: vec![].into(),
            model: None,
            thinking_level: None,
            prompt_deliveries: None,
        })
    })
    .unwrap();
}

#[test]
fn only_confirmed_paginated_history_in_the_effective_home_requests_a_preview() {
    let other_home = tempfile::tempdir().unwrap();
    for (mode, config, env_override) in [
        ("legacy", Some(json!({"config":{}})), false),
        ("unknown", None, false),
        ("paginated", None, true),
        ("paginated", None, false),
        (
            "paginated",
            Some(json!({"config":{"sqlite_home":other_home.path()}})),
            false,
        ),
        (
            "paginated",
            Some(json!({"config":{"sqlite_home":"."}})),
            false,
        ),
        ("paginated", Some(json!({})), false),
    ] {
        let (home, db, _) = fixture();
        db.execute("UPDATE threads SET history_mode=?1", [mode])
            .unwrap();
        // Config checks run only after finding a supported backing store.
        if mode == "paginated" && (config.is_some() || env_override) {
            std::fs::write(home.path().join("thread_history_1.sqlite"), "").unwrap();
        }
        let mut scope = scope();
        if env_override {
            scope.env.push((
                "CODEX_SQLITE_HOME".into(),
                Some(other_home.path().as_os_str().into()),
            ));
        }
        let mut replies = Vec::new();
        if let Some(config) = config {
            replies.extend([json!({"data":[]}), config]);
        }
        replies
            .push(json!({"thread":{"turns":[{"items":[{"type":"agentMessage","text":"full"}]}]}}));
        let mut connection = super::super::CodexConnection::new(rpc_results(replies), Vec::new());
        let history = super::super::load_history_with_client(
            &mut connection,
            home.path(),
            "thread",
            scope,
            Some(&mut |_| panic!("unexpected preview for {mode}")),
        )
        .unwrap();
        assert_eq!(history.messages[0]["content"][0]["text"], "full");
        let sent = String::from_utf8(connection.into_writer()).unwrap();
        assert!(!sent.contains("thread/turns/list"), "{mode}: {sent}");
        assert!(sent.contains("thread/read"), "{mode}: {sent}");
    }
}

#[test]
fn preview_requires_an_indexed_thread_in_the_supported_paginated_store() {
    let (home, _db, rollout) = paginated_fixture();
    let history = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
    assert!(paginated_revision(home.path(), "thread").is_some());
    for version in ["state_6.sqlite", "thread_history_2.sqlite"] {
        let path = home.path().join(version);
        std::fs::write(&path, "").unwrap();
        assert!(paginated_revision(home.path(), "thread").is_none());
        std::fs::remove_file(path).unwrap();
    }
    let before = paginated_revision(home.path(), "thread").unwrap();
    history
        .execute(
            "INSERT INTO thread_history_projection_state VALUES ('other', 10, 1)",
            [],
        )
        .unwrap();
    assert!(paginated_revision(home.path(), "thread").unwrap() == before);
    history
        .execute(
            "UPDATE thread_history_projection_state SET next_rollout_ordinal=2
             WHERE thread_id='thread'",
            [],
        )
        .unwrap();
    assert!(paginated_revision(home.path(), "thread").unwrap() != before);
    std::fs::write(rollout, "new bytes outside the index").unwrap();
    assert!(paginated_revision(home.path(), "thread").is_none());
    std::fs::write(home.path().join("rollout.jsonl"), "initial rollout").unwrap();
    history
        .execute("UPDATE thread_turns SET thread_id='other'", [])
        .unwrap();
    assert!(paginated_revision(home.path(), "thread").is_none());
    history.execute("DROP TABLE thread_turns", []).unwrap();
    assert!(paginated_revision(home.path(), "thread").is_none());
}

#[test]
fn optional_recent_page_never_replaces_the_full_history_read() {
    let (home, _db, _) = paginated_fixture();
    let full = json!({"id":2,"result":{"thread":{"turns":[turn("final")]}}});
    for (page, expected_preview) in [
        (
            json!({"id":1,"result":{"data":[turn("recent")],"nextCursor":null}}),
            true,
        ),
        (
            json!({"id":1,"result":{"data":[],"nextCursor":null}}),
            false,
        ),
        (json!({"id":1,"result":{"data":[{"items":[]}]}}), false),
        (
            json!({"id":1,"error":{"code":-32601,"message":"method not found"}}),
            false,
        ),
    ] {
        let mut connection = super::super::CodexConnection::new(
            std::io::Cursor::new(format!("{page}\n{full}\n")),
            Vec::new(),
        );
        let mut previews = Vec::new();
        let history = super::super::read_history(
            &mut connection,
            home.path(),
            "thread",
            Some(&mut |messages| previews.push(messages)),
        )
        .unwrap();
        assert_eq!(previews.len(), usize::from(expected_preview));
        if expected_preview {
            assert_eq!(previews[0].content[0]["content"][0]["text"], "recent");
        }
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0]["content"][0]["text"], "final");
    }
}

#[test]
fn a_rollout_change_during_the_page_read_suppresses_the_callback() {
    struct ChangeOnRequest(PathBuf);
    impl std::io::Write for ChangeOnRequest {
        fn write(&mut self, request: &[u8]) -> std::io::Result<usize> {
            std::fs::write(&self.0, "changed after taking the initial cursor")?;
            Ok(request.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let (home, _db, rollout) = paginated_fixture();
    let replies = rpc_results([
        json!({"data":[turn("stale")]}),
        json!({"thread":{"turns":[turn("current")]}}),
    ]);
    let mut connection = super::super::CodexConnection::new(replies, ChangeOnRequest(rollout));
    let history = super::super::read_history(
        &mut connection,
        home.path(),
        "thread",
        Some(&mut |_| panic!("published stale history")),
    )
    .unwrap();
    assert_eq!(history.messages[0]["content"][0]["text"], "current");
}

#[test]
fn preview_followed_by_a_failed_full_read_does_not_fill_the_cache() {
    let (home, _db, _) = paginated_fixture();
    let page = json!({"id":1,"result":{"data":[turn("recent")]}});
    let error = json!({"id":2,"error":{"code":-1,"message":"full read failed"}});
    let mut connection =
        super::super::CodexConnection::new(Cursor::new(format!("{page}\n{error}\n")), Vec::new());
    let mut previews = Vec::new();
    let result = load(home.path(), "thread", scope(), || {
        super::super::read_history(
            &mut connection,
            home.path(),
            "thread",
            Some(&mut |messages| previews.push(messages)),
        )
    });
    assert!(result.is_err_and(|error| error.contains("full read failed")));
    assert_eq!(previews.len(), 1);
    let calls = Cell::new(0);
    counted_load(home.path(), scope(), &calls);
    assert_eq!(calls.get(), 1);
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
            messages: vec![].into(),
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
            json!({"data": [], "nextCursor":"more"}),
            json!({"config": {}}),
            false,
        ),
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
