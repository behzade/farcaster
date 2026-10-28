use super::*;
use serde_json::json;

#[test]
fn fork_evidence_requires_matching_identity_and_an_ordinary_source()
-> Result<(), Box<dyn std::error::Error>> {
    let home = tempfile::tempdir()?;
    let rollout = home.path().join("rollout.jsonl");
    let database = Connection::open(home.path().join("state_5.sqlite"))?;
    database
        .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL);")?;
    database.execute(
        "INSERT INTO threads VALUES('fork',?1)",
        [rollout.to_string_lossy()],
    )?;
    let path = Path::new("/locators/codex-cli/fork");
    for source in [
        json!("cli"),
        json!("vscode"),
        json!("exec"),
        json!("appServer"),
        json!("unknown"),
        Value::Null,
        json!({"subAgent": "review"}),
    ] {
        for key in ["forked_from_id", "parent_thread_id"] {
            let mut header =
                json!({"type": "session_meta", "payload": {"id": "fork", "source": source}});
            header["payload"][key] = json!("parent");
            std::fs::write(&rollout, format!("{header}\nnot a header\n"))?;
            let expected = source
                .as_str()
                .filter(|s| *s != "unknown")
                .map(|_| "parent".to_owned());
            assert_eq!(read(path, Some(home.path())), expected);
        }
    }
    for payload in [
        json!({"id": "other", "source": "cli", "forked_from_id": "parent"}),
        json!({"id": "fork", "source": "cli", "forked_from_id": "fork"}),
        json!({"id": "fork", "source": "cli", "forked_from_id": ""}),
        json!({"id": "fork", "source": "cli"}),
    ] {
        std::fs::write(
            &rollout,
            json!({"type": "session_meta", "payload": payload}).to_string(),
        )?;
        assert_eq!(read(path, Some(home.path())), None);
    }
    std::fs::write(&rollout, "invalid json\n")?;
    assert_eq!(read(path, Some(home.path())), None);
    std::fs::remove_file(&rollout)?;
    assert_eq!(read(path, Some(home.path())), None);
    let missing = home.path().join("missing-home");
    assert_eq!(read(path, Some(&missing)), None);
    assert!(!missing.exists());
    let profiled =
        Path::new("/locators/profiles/11111111-1111-4111-8111-111111111111/codex-cli/fork");
    assert_eq!(read(profiled, None), None);
    Ok(())
}
