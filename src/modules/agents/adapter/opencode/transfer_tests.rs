use std::{collections::VecDeque, path::PathBuf, time::SystemTime};

use serde_json::{Value, json};

use super::super::contract::{OpenCodeHttpMethod, OpenCodeHttpRequest, OpenCodeHttpResponse};
use super::*;

struct Transport {
    replies: VecDeque<Result<Value, String>>,
    writes: Vec<(String, Value)>,
}

impl OpenCodeHttpTransport for Transport {
    fn execute(&mut self, request: OpenCodeHttpRequest) -> Result<OpenCodeHttpResponse, String> {
        if request.method == OpenCodeHttpMethod::Post {
            self.writes.push((
                request.path,
                serde_json::from_slice(request.body.as_deref().expect("body")).expect("JSON"),
            ));
        }
        let data = self.replies.pop_front().expect("unexpected request")?;
        Ok(OpenCodeHttpResponse {
            status: if data.is_null() { 204 } else { 200 },
            body: if data.is_null() {
                vec![]
            } else {
                serde_json::to_vec(&json!({"data":data})).expect("JSON")
            },
        })
    }
}

fn stored(id: &str, directory: &str) -> Result<Value, String> {
    Ok(json!({"id":id,"location":{"directory":directory}}))
}

fn family() -> Vec<SessionSummary> {
    ["root", "child"]
        .into_iter()
        .map(|id| {
            SessionSummary::from_cached_for_harness(
                id.into(),
                "opencode2".into(),
                PathBuf::from(format!("/locators/opencode2/{id}")),
                "/source".into(),
                String::new(),
                String::new(),
                String::new(),
                (id == "child").then(|| "/locators/opencode2/root".into()),
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

fn client(replies: impl IntoIterator<Item = Result<Value, String>>) -> OpenCodeClient<Transport> {
    OpenCodeClient::new(Transport {
        replies: replies.into_iter().collect(),
        writes: vec![],
    })
}

#[test]
fn waits_for_delivered_moves_and_keeps_locator_identity() {
    let family = family();
    let mut client = client([
        stored("root", "/source"),
        Ok(json!([])),
        stored("child", "/source"),
        Ok(json!([])),
        Ok(Value::Null),
        Ok(json!([{"type":"move"}])),
        stored("root", "/destination"),
        Ok(json!([])),
        stored("root", "/destination"),
        Ok(Value::Null),
        Ok(json!([])),
        stored("child", "/destination"),
    ]);
    let moved = move_with_client(&mut client, &family, "/destination", Duration::from_secs(1))
        .expect("move");
    assert_eq!(moved.root, family[0].path);
    for session in family {
        assert_eq!(moved.paths.get(&session.path), Some(&session.path));
    }
    let transport = client.into_transport();
    assert!(transport.replies.is_empty());
    assert_eq!(
        transport.writes,
        vec![
            (
                "/api/session/root/move".into(),
                json!({"directory":"/destination"})
            ),
            (
                "/api/session/child/move".into(),
                json!({"directory":"/destination"})
            ),
        ]
    );
}

#[test]
fn queued_work_in_last_member_prevents_all_moves() {
    let mut client = client([
        stored("root", "/source"),
        Ok(json!([])),
        stored("child", "/source"),
        Ok(json!([{"type":"user"}])),
    ]);
    assert!(
        move_with_client(&mut client, &family(), "/destination", Duration::ZERO)
            .expect_err("queued work")
            .contains("queued OpenCode work")
    );
    assert!(client.into_transport().writes.is_empty());
}

#[test]
fn lost_response_restores_every_attempted_member_in_reverse_order() {
    let mut client = client([
        stored("root", "/source"),
        Ok(json!([])),
        stored("child", "/source"),
        Ok(json!([])),
        Ok(Value::Null),
        Ok(json!([])),
        stored("root", "/destination"),
        Err("connection lost".into()),
        Ok(Value::Null),
        Ok(json!([])),
        stored("child", "/source"),
        Ok(Value::Null),
        Ok(json!([])),
        stored("root", "/source"),
    ]);
    assert!(
        move_with_client(&mut client, &family(), "/destination", Duration::ZERO)
            .expect_err("failed move")
            .contains("original folders restored")
    );
    let transport = client.into_transport();
    assert!(transport.replies.is_empty());
    assert_eq!(
        &transport.writes[2..],
        &[
            (
                "/api/session/child/move".into(),
                json!({"directory":"/source"})
            ),
            (
                "/api/session/root/move".into(),
                json!({"directory":"/source"})
            ),
        ]
    );
}

#[test]
fn timeout_does_not_claim_rollback_while_a_move_is_still_queued() {
    let mut client = client([
        stored("root", "/source"),
        Ok(json!([])),
        Ok(Value::Null),
        Ok(json!([{"type":"move"}])),
        stored("root", "/source"),
        Ok(Value::Null),
        Ok(json!([{"type":"move"}])),
        stored("root", "/source"),
    ]);
    let error = move_with_client(&mut client, &family()[..1], "/destination", Duration::ZERO)
        .expect_err("pending move");
    assert!(error.contains("Could not confirm restored folders"));
    assert!(!error.contains("original folders restored"));
}

#[test]
#[ignore = "requires an installed opencode2 and a local listening socket; uses isolated storage"]
fn native_family_move_survives_server_restart() -> Result<(), Box<dyn std::error::Error>> {
    use super::super::server::OpenCodeServerProcess;
    use std::process::{Command, Stdio};

    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&destination)?;
    let start = || -> Result<OpenCodeServerProcess, Box<dyn std::error::Error>> {
        let child = Command::new("opencode2")
            .args(["serve", "--stdio"])
            .current_dir(temp.path())
            .env("XDG_DATA_HOME", temp.path().join("data"))
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_STATE_HOME", temp.path().join("state"))
            .env("OPENCODE_SERVER_PASSWORD", "farcaster-move-test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(OpenCodeServerProcess::attach(
            child,
            "opencode",
            "farcaster-move-test",
        )?)
    };
    let mut server = start()?;
    let mut client = server.client();
    let root = client.create_session(source.to_str().ok_or("source path")?, None, None)?;
    let child =
        client.create_session(source.to_str().ok_or("source path")?, Some(&root.id), None)?;
    let mut family = family();
    for (summary, stored) in family.iter_mut().zip([&root, &child]) {
        summary.id = stored.id.clone();
        summary.path = temp.path().join("opencode2").join(&stored.id);
        summary.project = source.clone();
    }
    let moved = move_with_client(
        &mut client,
        &family,
        destination.to_str().ok_or("destination path")?,
        Duration::from_secs(10),
    )?;
    assert_eq!(moved.paths.len(), 2);
    server.terminate()?;
    let mut server = start()?;
    for original in [&root, &child] {
        let stored = server.client().get_session(&original.id)?;
        assert_eq!(Path::new(&stored.location.directory), destination);
        assert_eq!(stored.parent_id, original.parent_id);
        assert!(server.client().session_inbox(&original.id)?.is_empty());
    }
    for original in [child, root] {
        server.client().delete_session(&original.id)?;
    }
    server.terminate()?;
    Ok(())
}
