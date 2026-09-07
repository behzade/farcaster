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

fn preflight(id: &str) -> Vec<Value> {
    vec![
        json!({"thread":{"id":id,"cwd":"/source","status":{"type":"notLoaded"}}}),
        json!({"data":[]}),
        json!({"goal":null}),
    ]
}

fn connection<'a>(
    results: Vec<Value>,
    notifications: &[(usize, &str, &str)],
    sent: &'a mut Vec<u8>,
) -> CodexConnection<Cursor<Vec<u8>>, &'a mut Vec<u8>> {
    let mut wire = Vec::new();
    for (index, result) in results.into_iter().enumerate() {
        let frame = if result.get("error").is_some() {
            json!({"id":index+1,"error":result["error"]})
        } else {
            json!({"id":index+1,"result":result})
        };
        writeln!(wire, "{frame}").expect("frame");
        for (_, id, cwd) in notifications
            .iter()
            .filter(|(after, _, _)| *after == index + 1)
        {
            let notification = json!({"method":"thread/settings/updated","params":{"threadId":id,"threadSettings":{"cwd":cwd}}});
            writeln!(wire, "{notification}").expect("notification");
        }
    }
    CodexConnection::new(Cursor::new(wire), sent)
}

#[test]
fn family_move_requires_matching_completion_notifications() {
    let mut results = preflight("root");
    results.extend(preflight("child"));
    results.extend([json!({}), json!({}), json!({}), json!({})]);
    let mut sent = Vec::new();
    let mut connection = connection(
        results,
        &[
            (9, "other", "/destination"),
            (9, "root", "/destination"),
            (10, "child", "/destination"),
        ],
        &mut sent,
    );
    let family = family();
    let moved = move_with_client(&mut connection, &family, "/destination").expect("move");
    assert_eq!(moved.root, family[0].path);
    for session in family {
        assert_eq!(moved.paths.get(&session.path), Some(&session.path));
    }
}

#[test]
fn queued_work_prevents_loading_or_moving_any_thread() {
    let mut results = preflight("root");
    results.extend([
        preflight("child")[0].clone(),
        json!({"data":[{"id":"queued"}]}),
    ]);
    let mut sent = Vec::new();
    let mut connection = connection(results, &[], &mut sent);
    assert!(
        move_with_client(&mut connection, &family(), "/destination")
            .expect_err("queued")
            .contains("queued Codex work")
    );
    drop(connection);
    assert!(!String::from_utf8(sent).unwrap().contains("thread/resume"));
}

#[test]
fn partial_failure_restores_both_attempted_threads() {
    let mut results = preflight("root");
    results.extend(preflight("child"));
    results.extend([
        json!({}),
        json!({}),
        json!({}),
        json!({"error":{"code":-32600,"message":"move failed"}}),
        json!({}),
        json!({}),
    ]);
    let mut sent = Vec::new();
    let mut connection = connection(
        results,
        &[
            (9, "root", "/destination"),
            (11, "child", "/source"),
            (12, "root", "/source"),
        ],
        &mut sent,
    );
    assert!(
        move_with_client(&mut connection, &family(), "/destination")
            .expect_err("failed move")
            .contains("original folders restored")
    );
    drop(connection);
    let updates: Vec<Value> = String::from_utf8(sent)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|frame| frame["method"] == "thread/settings/update")
        .map(|frame| frame["params"].clone())
        .collect();
    assert_eq!(
        updates,
        vec![
            json!({"threadId":"root","cwd":"/destination"}),
            json!({"threadId":"child","cwd":"/destination"}),
            json!({"threadId":"child","cwd":"/source"}),
            json!({"threadId":"root","cwd":"/source"}),
        ]
    );
}

#[test]
fn rpc_acknowledgement_without_notification_is_not_success() {
    let mut sent = Vec::new();
    let mut connection = connection(vec![json!({})], &[], &mut sent);
    assert!(update_cwd(&mut connection, "root", "/destination").is_err());
}

#[test]
#[ignore = "requires installed Codex; uses a disposable rollout and isolated SQLite state, no model calls"]
fn native_move_survives_restart() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&destination)?;
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let id = format!(
        "{:08x}-0000-4000-8000-{:012x}",
        std::process::id(),
        nonce & 0xffffffffffff
    );
    let path = temp.path().join(format!("{id}.jsonl"));
    let timestamp = "2026-09-07T00:00:00Z";
    let text = "Offline fixture; do not send a model request.";
    let header = json!({"timestamp":timestamp,"type":"session_meta","payload":{"id":id,"timestamp":timestamp,"cwd":source,"originator":"farcaster-test","cli_version":"0.0.0","source":"cli","model_provider":"openai"}});
    let message = json!({"timestamp":timestamp,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}});
    let event = json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"user_message","message":text,"images":[],"local_images":[]}});
    std::fs::write(&path, format!("{header}\n{message}\n{event}\n"))?;
    let command = || {
        let mut command = Command::new("codex");
        command
            .current_dir(temp.path())
            .env("CODEX_SQLITE_HOME", temp.path().join("sqlite"));
        command
    };
    let resume = json!({"threadId":id,"path":path,"sandbox":"read-only","approvalPolicy":"never"});
    with_server(command(), |connection| {
        request(connection, "thread/resume", resume.clone())?;
        let mut session = family().remove(0);
        session.id = id.clone();
        session.path = temp.path().join("codex-cli").join(&id);
        session.project = source.clone();
        move_with_client(
            connection,
            &[session],
            destination.to_str().ok_or("destination")?,
        )?;
        Ok(())
    })?;
    with_server(command(), |connection| {
        let resumed = request(connection, "thread/resume", resume)?;
        assert_eq!(resumed["cwd"].as_str(), destination.to_str());
        assert_eq!(resumed["thread"]["cwd"].as_str(), destination.to_str());
        assert_eq!(resumed["thread"]["id"].as_str(), Some(id.as_str()));
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
