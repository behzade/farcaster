use std::io::{BufReader, Cursor};

use serde_json::{Value, json};

use super::{
    connection::CodexConnection,
    contract::{CodexClientInfo, CodexInbound, CodexRequestId, CodexUserInput},
};

#[test]
fn native_vertical_slice_preserves_streaming_events() -> Result<(), String> {
    let input = [
        json!({
            "method": "item/agentMessage/delta",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1",
                "delta": "hello"
            }
        }),
        json!({
            "id": 1,
            "result": {
                "userAgent": "farcaster/0.149.0",
                "codexHome": "/tmp/codex",
                "platformFamily": "unix",
                "platformOs": "macos"
            }
        }),
        json!({
            "id": 2,
            "result": {
                "thread": {
                    "id": "thread-1",
                    "sessionId": "session-1",
                    "parentThreadId": null,
                    "cwd": "/project"
                }
            }
        }),
        json!({
            "id": 3,
            "result": {
                "thread": {
                    "id": "thread-1",
                    "sessionId": "session-1",
                    "parentThreadId": null,
                    "cwd": "/project"
                }
            }
        }),
        json!({
            "id": 4,
            "result": {
                "thread": {
                    "id": "thread-1",
                    "sessionId": "session-1",
                    "parentThreadId": null,
                    "cwd": "/project"
                }
            }
        }),
        json!({
            "id": 5,
            "result": {"turn": {"id": "turn-1", "status": "inProgress"}}
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {"id": "turn-1", "status": "completed"}
            }
        }),
        json!({"id": 6, "result": {}}),
    ]
    .into_iter()
    .map(|message| serde_json::to_string(&message).map(|line| format!("{line}\n")))
    .collect::<Result<String, _>>()
    .map_err(|error| error.to_string())?;
    let reader = BufReader::new(Cursor::new(input.into_bytes()));
    let writer = Vec::new();
    let mut connection = CodexConnection::new(reader, writer);

    let initialized = connection.initialize(CodexClientInfo {
        name: "farcaster".into(),
        title: Some("Farcaster".into()),
        version: "0.1.0".into(),
    })?;
    assert_eq!(initialized.platform_family, "unix");
    let thread = connection.start_thread("/project", None, None)?;
    assert_eq!(thread.id, "thread-1");
    assert_eq!(
        connection
            .fork_thread("thread-1", "/project", None, None)?
            .id,
        "thread-1"
    );
    assert_eq!(connection.resume_thread("thread-1")?.id, "thread-1");
    let turn = connection.start_turn(
        "thread-1",
        vec![
            CodexUserInput::text("hello"),
            CodexUserInput::Image {
                url: "data:image/png;base64,aGVsbG8=".into(),
            },
            CodexUserInput::LocalImage {
                path: "/tmp/image.png".into(),
            },
        ],
    )?;
    assert_eq!(turn.id, "turn-1");

    assert_eq!(
        connection.next()?,
        CodexInbound::Notification {
            method: "item/agentMessage/delta".into(),
            params: json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1",
                "delta": "hello"
            }),
        }
    );
    assert!(matches!(
        connection.next()?,
        CodexInbound::Notification { method, .. } if method == "turn/completed"
    ));
    connection.interrupt_turn("thread-1", "turn-1")?;

    let output = String::from_utf8(connection.into_writer()).map_err(|error| error.to_string())?;
    let sent = output
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({"method":"initialized"}));
    assert_eq!(sent[2]["method"], "thread/start");
    assert_eq!(sent[3]["method"], "thread/fork");
    assert_eq!(sent[4]["method"], "thread/resume");
    assert_eq!(sent[5]["method"], "turn/start");
    assert_eq!(sent[5]["params"]["input"][0]["text_elements"], json!([]));
    assert_eq!(sent[5]["params"]["input"][1]["type"], "image");
    assert_eq!(sent[6]["method"], "turn/interrupt");
    Ok(())
}

#[test]
fn server_requests_can_be_answered_without_normalizing_them() -> Result<(), String> {
    let input = br#"{"id":"approval-1","method":"item/commandExecution/requestApproval","params":{"command":"rm file"}}
"#;
    let reader = BufReader::new(Cursor::new(input.to_vec()));
    let writer = Vec::new();
    let mut connection = CodexConnection::new(reader, writer);
    let request = connection.next()?;
    let CodexInbound::ServerRequest { id, method, .. } = request else {
        return Err("expected server request".into());
    };
    assert_eq!(method, "item/commandExecution/requestApproval");
    connection.respond(&id, json!({"decision":"decline"}))?;
    assert_eq!(id, CodexRequestId::String("approval-1".into()));
    assert_eq!(
        String::from_utf8(connection.into_writer()).map_err(|error| error.to_string())?,
        "{\"id\":\"approval-1\",\"result\":{\"decision\":\"decline\"}}\n"
    );
    Ok(())
}
