use super::*;

fn history(complete: bool) -> DiscoveredHistory {
    DiscoveredHistory {
        messages: vec![serde_json::json!({"role": "user", "content": "cached"})],
        model: Some(("provider".into(), "model".into())),
        thinking_level: Some("high".into()),
        prompt_deliveries: complete.then(|| farcaster_sessions::PromptDeliveryReconciliation {
            absence_is_not_delivered: true,
            delivered: vec!["opencode-delivered".into()],
            pending: vec!["opencode-pending".into()],
        }),
    }
}

#[cfg(unix)]
#[test]
fn path_discovery_is_bounded_and_failed_commands_fall_back() {
    let mut success = Command::new("sh");
    success.args(["-c", "printf 'db /fixture/history.db\\n'"]);
    assert_eq!(
        discover_paths(&mut success, Duration::from_secs(2)).unwrap(),
        b"db /fixture/history.db\n"
    );
    let mut failed = Command::new("sh");
    failed.args(["-c", "exit 1"]);
    assert!(discover_paths(&mut failed, Duration::from_secs(2)).is_none());
    let mut stuck = Command::new("sh");
    stuck.args(["-c", "while :; do :; done"]);
    assert!(discover_paths(&mut stuck, Duration::from_millis(30)).is_none());
}

#[test]
fn reported_path_preserves_profile_and_database_overrides() {
    assert_eq!(
        parse_database_path(
            "data       /profiles/one\ndb         /profiles/one/custom db.sqlite\n"
        ),
        Some(PathBuf::from("/profiles/one/custom db.sqlite"))
    );
    for output in [
        "",
        "db :memory:\n",
        "db relative.db\n",
        "db /one\ndb /two\n",
        "database /one\n",
    ] {
        assert!(parse_database_path(output).is_none());
    }
}

#[test]
fn unchanged_database_skips_loader_and_preserves_the_whole_result() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    std::fs::write(&db, "database").unwrap();
    let cache = HistoryCache::new();
    let key = (PathBuf::from("opencode"), db, "session".into());
    load_database(&cache, key.clone(), || Ok(history(true))).unwrap();
    let hit = load_database(&cache, key, || panic!("history loader must be skipped")).unwrap();
    assert_eq!(hit.messages, history(true).messages);
    assert_eq!(hit.model, history(true).model);
    assert_eq!(hit.thinking_level, history(true).thinking_level);
    let deliveries = hit.prompt_deliveries.unwrap();
    assert_eq!(deliveries.pending, ["opencode-pending"]);
    assert_eq!(deliveries.delivered, ["opencode-delivered"]);
}

#[test]
fn main_wal_and_missing_files_invalidate_history() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    let wal = sidecar(&db, "-wal");
    let cache = HistoryCache::new();
    let key = (PathBuf::from("opencode"), db.clone(), "session".into());
    let calls = Cell::new(0);
    let read = || {
        load_database(&cache, key.clone(), || {
            calls.set(calls.get() + 1);
            Ok(history(true))
        })
        .unwrap();
    };
    std::fs::write(&db, "database").unwrap();
    read();
    read();
    assert_eq!(calls.get(), 1);
    // Model, message and inbox changes may all land in the WAL only.
    std::fs::write(&wal, "inbox").unwrap();
    read();
    std::fs::write(&wal, "inbox and model").unwrap();
    read();
    std::fs::remove_file(&wal).unwrap();
    read();
    std::fs::write(&db, "checkpointed database").unwrap();
    read();
    std::fs::remove_file(&db).unwrap();
    read();
    read();
    assert_eq!(calls.get(), 7);
}

#[test]
fn committed_sqlite_wal_updates_invalidate_without_a_main_file_write() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
         CREATE TABLE fixture(model TEXT, pending TEXT);
         INSERT INTO fixture VALUES ('first', 'pending-one');",
        )
        .unwrap();
    let cache = HistoryCache::new();
    let key = (PathBuf::from("opencode"), db.clone(), "session".into());
    let calls = Cell::new(0);
    let read = || {
        load_database(&cache, key.clone(), || {
            calls.set(calls.get() + 1);
            let (model, pending): (String, String) = connection
                .query_row("SELECT model, pending FROM fixture", [], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap();
            let mut result = history(true);
            result.model = Some(("provider".into(), model));
            result.prompt_deliveries.as_mut().unwrap().pending = vec![pending];
            Ok(result)
        })
        .unwrap()
    };
    let first = read();
    assert_eq!(read().model, first.model);
    assert_eq!(calls.get(), 1);
    let main_before = FileStamp::read(&db).unwrap();
    connection
        .execute(
            "UPDATE fixture SET model='second', pending='pending-two'",
            [],
        )
        .unwrap();
    assert!(FileStamp::read(&db).as_ref() == Some(&main_before));
    let next = read();
    assert_eq!(calls.get(), 2);
    assert_eq!(next.model, Some(("provider".into(), "second".into())));
    assert_eq!(next.prompt_deliveries.unwrap().pending, ["pending-two"]);
}

#[test]
fn unknown_freshness_and_failed_inbox_are_never_cached() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("opencode.db");
    std::fs::write(&db, "database").unwrap();
    let cache = HistoryCache::new();
    let key = (PathBuf::from("opencode"), db.clone(), "session".into());
    for _ in 0..2 {
        let result = load_database(&cache, key.clone(), || Ok(history(false))).unwrap();
        assert!(result.prompt_deliveries.is_none());
    }
    assert!(load_database(&cache, key.clone(), || Err("inbox retry".into())).is_err());
    std::fs::write(sidecar(&db, "-journal"), "transaction").unwrap();
    load_database(&cache, key.clone(), || Ok(history(true))).unwrap();
    assert!(load_database(&cache, key, || Err("uncached".into())).is_err());
}

#[test]
fn changed_during_load_and_different_profiles_do_not_hit() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("one.db");
    let other = dir.path().join("two.db");
    std::fs::write(&db, "database").unwrap();
    std::fs::write(&other, "database").unwrap();
    let cache = HistoryCache::new();
    let key = (PathBuf::from("opencode"), db.clone(), "session".into());
    load_database(&cache, key.clone(), || {
        std::fs::write(sidecar(&db, "-wal"), "new messages").unwrap();
        Ok(history(true))
    })
    .unwrap();
    assert!(load_database(&cache, key.clone(), || Err("changed".into())).is_err());
    load_database(&cache, key, || Ok(history(true))).unwrap();
    let other_key = (PathBuf::from("opencode"), other, "session".into());
    assert!(load_database(&cache, other_key, || Err("other profile".into())).is_err());
}
