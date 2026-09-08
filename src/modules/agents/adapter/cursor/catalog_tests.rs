use super::*;

fn fixture(root: &Path, id: &str, persisted: bool) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let cwd = std::env::current_dir().unwrap();
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "cwd": cwd, "title": "ACP fixture", "futureField": true
        }))
        .unwrap(),
    )
    .unwrap();
    if persisted {
        let db = Connection::open(dir.join("store.db")).unwrap();
        db.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);")
            .unwrap();
        db.execute(
            "INSERT INTO meta VALUES ('0', ?1)",
            [encode_hex(
                br#"{"name":"ACP fixture","blobEncryptionKey":"preserve-me"}"#,
            )],
        )
        .unwrap();
    }
    dir
}

#[test]
fn config_directory_matches_cursor_precedence() {
    assert_eq!(
        config_root(Some("/override".into()), Some("/xdg".into()), None).unwrap(),
        PathBuf::from("/override")
    );
    assert_eq!(
        config_root(None, Some("/xdg".into()), None).unwrap(),
        PathBuf::from("/xdg/cursor")
    );
    assert_eq!(
        config_root(None, None, Some("/home/user".into())).unwrap(),
        PathBuf::from("/home/user/.cursor")
    );
    assert!(config_root(None, None, None).is_err());
}

#[test]
fn acp_catalog_excludes_unpersisted_drafts_and_uses_sidecar() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), "persisted", true);
    let draft = fixture(root.path(), "draft", false);
    assert!(session_data(&draft).unwrap().1);
    let sessions = discover_at(root.path(), root.path(), "ACP fixture").unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "persisted");
    assert_eq!(sessions[0].project, std::env::current_dir().unwrap());
    assert!(
        discover_at(root.path(), root.path(), "does-not-match")
            .unwrap()
            .is_empty()
    );
    assert!(find_session_at(root.path(), "missing").is_err());
    assert!(find_session_at(root.path(), "../escape").is_err());
    std::fs::write(draft.join("meta.json"), b"broken").unwrap();
    assert!(session_data(&draft).is_err());
}

#[test]
fn rename_updates_database_and_sidecar_without_losing_fields() {
    let root = tempfile::tempdir().unwrap();
    let dir = fixture(root.path(), "persisted", true);
    rename_at(&dir, "Renamed").unwrap();
    let meta = metadata(&dir).unwrap();
    assert_eq!(meta.title.as_deref(), Some("Renamed"));
    assert_eq!(meta.extra["futureField"], true);
    let db = Connection::open(dir.join("store.db")).unwrap();
    let encoded: String = db
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&decode_hex(&encoded).unwrap()).unwrap();
    assert_eq!(stored["name"], "Renamed");
    assert_eq!(stored["blobEncryptionKey"], "preserve-me");
    let draft = fixture(root.path(), "draft", false);
    rename_at(&draft, "Draft title").unwrap();
    assert!(session_data(&draft).unwrap().1);
}

#[cfg(unix)]
#[test]
fn refuses_symlink_sessions_and_databases() {
    let root = tempfile::tempdir().unwrap();
    let dir = fixture(root.path(), "real", true);
    std::os::unix::fs::symlink(&dir, root.path().join("link")).unwrap();
    assert!(find_session_at(root.path(), "link").is_err());
    let draft = fixture(root.path(), "draft", false);
    std::os::unix::fs::symlink(dir.join("store.db"), draft.join("store.db")).unwrap();
    assert!(session_data(&draft).is_err());
}
