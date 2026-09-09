use std::{io::Cursor, path::PathBuf, time::SystemTime};

use super::*;

fn family() -> Vec<SessionSummary> {
    ["root", "child"]
        .into_iter()
        .map(|id| {
            SessionSummary::from_cached_for_harness(
                id.into(),
                "codex-cli".into(),
                PathBuf::from(format!("/locators/codex-cli/{id}")),
                "/source".into(),
                String::new(),
                String::new(),
                String::new(),
                None,
                SystemTime::now(),
                0,
                Default::default(),
                false,
                false,
                String::new(),
            )
        })
        .collect()
}

#[test]
fn family_preflight_rejects_pending_work() {
    for (index, blocked, expected) in [
        (
            0,
            json!({"thread":{"id":"child","status":{"type":"active"}}}),
            "must be idle",
        ),
        (1, json!({"data":[{"id":"queued"}]}), "queued Codex work"),
        (
            2,
            json!({"goal":{"status":"active"}}),
            "Pause the Codex goal",
        ),
    ] {
        let responses = |id| {
            vec![
                json!({"thread":{"id":id,"path":format!("/{id}.jsonl"),"status":{"type":"idle"}}}),
                json!({"data":[]}),
                json!({"goal":null}),
            ]
        };
        let mut results = responses("root");
        let mut child = responses("child");
        child[index] = blocked;
        results.extend(child);
        let mut wire = Vec::new();
        for (index, result) in results.into_iter().enumerate() {
            writeln!(wire, "{}", json!({"id":index+1,"result":result}))
                .expect("test operation should succeed");
        }
        let mut connection = CodexConnection::new(Cursor::new(wire), Vec::new());
        assert!(
            inspect_family(&mut connection, &family())
                .expect_err("invalid test input must fail")
                .contains(expected)
        );
    }
}

#[test]
#[ignore = "requires installed Codex; uses a disposable rollout and isolated SQLite state, no model calls"]
fn native_move_survives_restart() -> Result<(), Box<dyn std::error::Error>> {
    // The catalog deliberately hides projects under the system temp directory.
    let temp = tempfile::tempdir_in(std::env::current_dir()?)?;
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&destination)?;
    let source = source.canonicalize()?;
    let destination = destination.canonicalize()?;
    std::fs::create_dir(temp.path().join("home"))?;
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let id = format!(
        "{:08x}-0000-4000-8000-{:012x}",
        std::process::id(),
        nonce & 0xffffffffffff
    );
    let sessions = temp.path().join("home/sessions/2026/09/07");
    std::fs::create_dir_all(&sessions)?;
    let path = sessions.join(format!("rollout-2026-09-07T00-00-00-{id}.jsonl"));
    let timestamp = "2026-09-07T00:00:00Z";
    let text = "Offline fixture; do not send a model request.";
    let header = json!({"timestamp":timestamp,"type":"session_meta","payload":{"id":id,"timestamp":timestamp,"cwd":source,"originator":"farcaster-test","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
    let message = json!({"timestamp":timestamp,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}});
    let event = json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"user_message","message":text,"images":[],"local_images":[]}});
    std::fs::write(&path, format!("{header}\n{message}\n{event}\n"))?;
    let command = || {
        let mut command = Command::new(
            std::env::var_os("FARCASTER_CODEX_PATH").unwrap_or_else(|| "codex".into()),
        );
        command
            .current_dir(temp.path())
            .env("CODEX_HOME", temp.path().join("home"))
            .env("CODEX_SQLITE_HOME", temp.path().join("sqlite"));
        command
    };
    let resume = json!({"threadId":id,"path":path,"sandbox":"read-only","approvalPolicy":"never"});
    with_server(command(), |connection, _| {
        request(connection, "thread/resume", resume.clone())?;
        Ok(())
    })?;
    // Production starts a fresh server: do not preload the thread in this process.
    {
        let mut session = family().remove(0);
        session.id = id.clone();
        session.path = temp.path().join("codex-cli").join(&id);
        session.project = source.clone();
        move_via_server(
            command(),
            &[session],
            destination.to_str().ok_or("destination")?,
        )?;
    }
    std::fs::remove_dir(&source)?;
    with_server(command(), |connection, home| {
        let listed = request(connection, "thread/list", json!({"limit":100}))?;
        let threads = listed["data"].as_array().ok_or("missing thread list")?;
        assert_eq!(threads.len(), 1, "move created a duplicate: {listed}");
        assert_eq!(threads[0]["id"].as_str(), Some(id.as_str()));
        // Native history keeps its original folder; Farcaster owns the move.
        assert_eq!(threads[0]["cwd"].as_str(), source.to_str());
        let discovered =
            super::super::catalog::discover_with_client(connection, home, temp.path(), "")?;
        assert_eq!(
            discovered.len(),
            1,
            "Farcaster discovery duplicated the session"
        );
        assert_eq!(discovered[0].id, id);
        let project = discovered[0].project.clone();
        assert_eq!(project, destination);
        // Resume by identity, without the fixture path that could mask discovery failures.
        let resumed = connection.resume_thread(&id, crate::agents::HarnessAccessMode::Sandboxed)?;
        assert_eq!(resumed.cwd, destination.to_str().ok_or("destination")?);
        assert_eq!(resumed.id, id);
        let stored = request(
            connection,
            "thread/read",
            json!({"threadId":id,"includeTurns":true}),
        )?;
        assert!(stored.to_string().contains(text));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn persisted_move_preserves_history_and_rejects_invalid_family_before_writing()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = project_database(temp.path());
    let path = temp.path().join("root.jsonl");
    let body =
        "{\"type\":\"response_item\",\"payload\":{\"text\":\"history stays byte-for-byte\"}}\n";
    let original = format!(
        "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"root\",\"cwd\":\"/source\",\"unknown\":true}}}}\n{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":\"/older\",\"model\":\"keep\"}}}}\n{body}{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":\"/source\",\"model\":\"keep\"}}}}\n"
    );
    std::fs::write(&path, &original)?;
    let members = vec![
        ("root".into(), path.clone()),
        ("missing".into(), temp.path().join("missing")),
    ];
    assert!(persist_folders(&database, &members, "/destination").is_err());
    assert_eq!(std::fs::read_to_string(&path)?, original);
    assert!(!database.exists());
    persist_folders(&database, &members[..1], "/destination")?;
    let moved = std::fs::read_to_string(&path)?;
    assert_eq!(moved, original);
    assert_eq!(
        saved_project(&database, "root")?,
        Some("/destination".into())
    );
    assert_eq!(saved_project(&database, "missing")?, None);
    Ok(())
}

#[test]
fn move_preserves_history_appended_by_an_already_open_writer()
-> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    let temp = tempfile::tempdir()?;
    let database = project_database(temp.path());
    let path = temp.path().join("root.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"root\",\"cwd\":\"/source\"}}\n",
    )?;
    // Codex's rollout recorder keeps an append handle open. Keeping it open
    // across the move reproduces the race without threads, sleeps, or hooks.
    let mut writer = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(&path)?;
    let before = "{\"type\":\"response_item\",\"payload\":{\"text\":\"before move\"}}\n";
    let after = "{\"type\":\"response_item\",\"payload\":{\"text\":\"after move\"}}\n";
    writer.write_all(before.as_bytes())?;
    writer.sync_all()?;
    let moved = persist_folders(&database, &[("root".into(), path.clone())], "/destination");
    writer.write_all(after.as_bytes())?;
    writer.sync_all()?;

    // Prove the writer did write the entry before checking the discoverable file.
    writer.seek(SeekFrom::Start(0))?;
    let mut written = String::new();
    writer.read_to_string(&mut written)?;
    assert!(
        written.contains(after),
        "append handle did not write the fixture entry"
    );
    let discovered = std::fs::read_to_string(&path)?;
    assert!(discovered.contains(before));
    // Refusing a move with an open writer is acceptable; losing its write is not.
    assert!(
        discovered.contains(after),
        "move result {moved:?}: append succeeded, but history at the rollout path lost the entry"
    );
    Ok(())
}
