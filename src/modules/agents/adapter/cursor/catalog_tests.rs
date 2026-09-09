use super::*;

fn fixture(root: &Path, id: &str, persisted: bool) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).expect("test operation should succeed");
    let cwd = std::env::current_dir().expect("test operation should succeed");
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "cwd": cwd, "title": "ACP fixture", "futureField": true
        }))
        .expect("test operation should succeed"),
    )
    .expect("test operation should succeed");
    if persisted {
        let db = Connection::open(dir.join("store.db")).expect("test operation should succeed");
        db.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);")
            .expect("test operation should succeed");
        db.execute(
            "INSERT INTO meta VALUES ('0', ?1)",
            [encode_hex(
                br#"{"name":"ACP fixture","blobEncryptionKey":"preserve-me"}"#,
            )],
        )
        .expect("test operation should succeed");
    }
    dir
}

#[test]
fn config_directory_matches_cursor_precedence() {
    assert_eq!(
        config_root(Some("/override".into()), Some("/xdg".into()), None)
            .expect("test operation should succeed"),
        PathBuf::from("/override")
    );
    assert_eq!(
        config_root(None, Some("/xdg".into()), None).expect("test operation should succeed"),
        PathBuf::from("/xdg/cursor")
    );
    assert_eq!(
        config_root(None, None, Some("/home/user".into())).expect("test operation should succeed"),
        PathBuf::from("/home/user/.cursor")
    );
    assert!(config_root(None, None, None).is_err());
}

#[test]
fn acp_catalog_maps_protocol_entries_and_keeps_storage_checks_scoped() {
    let root = tempfile::tempdir().expect("test operation should succeed");
    fixture(root.path(), "persisted", true);
    let draft = fixture(root.path(), "draft", false);
    assert!(
        session_data(&draft)
            .expect("test operation should succeed")
            .1
    );
    let entry = serde_json::json!({"sessionId":"persisted","cwd":std::env::current_dir().expect("test operation should succeed"),
        "title":"ACP fixture","updatedAt":"2026-09-09T10:00:00Z"});
    let session =
        listed_session(root.path(), "acp fixture", &entry).expect("test operation should succeed");
    assert_eq!(session.id, "persisted");
    assert_eq!(
        session.project,
        std::env::current_dir().expect("test operation should succeed")
    );
    assert!(session.modified > UNIX_EPOCH);
    assert!(listed_session(root.path(), "does-not-match", &entry).is_none());
    let mut invalid = entry.clone();
    invalid["sessionId"] = "../escape".into();
    assert!(listed_session(root.path(), "", &invalid).is_none());
    assert!(find_session_at(root.path(), "missing").is_err());
    assert!(find_session_at(root.path(), "../escape").is_err());
    std::fs::write(draft.join("meta.json"), b"broken").expect("test operation should succeed");
    assert!(session_data(&draft).is_err());
}

#[test]
fn rename_updates_database_and_sidecar_without_losing_fields() {
    let root = tempfile::tempdir().expect("test operation should succeed");
    let dir = fixture(root.path(), "persisted", true);
    rename_at(&dir, "Renamed").expect("test operation should succeed");
    let meta = metadata(&dir).expect("test operation should succeed");
    assert_eq!(meta.title.as_deref(), Some("Renamed"));
    assert_eq!(meta.extra["futureField"], true);
    let db = Connection::open(dir.join("store.db")).expect("test operation should succeed");
    let encoded: String = db
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            row.get(0)
        })
        .expect("test operation should succeed");
    let stored: serde_json::Value =
        serde_json::from_slice(&decode_hex(&encoded).expect("test operation should succeed"))
            .expect("test operation should succeed");
    assert_eq!(stored["name"], "Renamed");
    assert_eq!(stored["blobEncryptionKey"], "preserve-me");
    let draft = fixture(root.path(), "draft", false);
    rename_at(&draft, "Draft title").expect("test operation should succeed");
    assert!(
        session_data(&draft)
            .expect("test operation should succeed")
            .1
    );
}

#[cfg(unix)]
#[test]
fn refuses_symlink_sessions_and_databases() {
    let root = tempfile::tempdir().expect("test operation should succeed");
    let dir = fixture(root.path(), "real", true);
    std::os::unix::fs::symlink(&dir, root.path().join("link"))
        .expect("test operation should succeed");
    assert!(find_session_at(root.path(), "link").is_err());
    let draft = fixture(root.path(), "draft", false);
    std::os::unix::fs::symlink(dir.join("store.db"), draft.join("store.db"))
        .expect("test operation should succeed");
    assert!(session_data(&draft).is_err());
}
