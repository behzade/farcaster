use std::path::{Path, PathBuf};

use super::*;
use crate::modules::agents::adapter::acp::wire::AcpInbound;

#[test]
fn catalog_exchange_times_out_when_agent_stalls() {
    let (release, stalled) = mpsc::channel::<()>();
    let error = run_with_timeout(Duration::from_millis(20), move || {
        let _ = stalled.recv();
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("timed out loading configuration"));
    drop(release);
}

#[test]
fn replay_keeps_messages_and_tools() {
    let notification = |update| AcpInbound::Notification {
        method: "session/update".into(),
        params: json!({"update":update}),
    };
    let history = replay_history([
        notification(
            json!({"sessionUpdate":"user_message_chunk","messageId":"u1","content":{"type":"text","text":"hello"}}),
        ),
        notification(
            json!({"sessionUpdate":"user_message_chunk","messageId":"u1","content":{"type":"text","text":" world"}}),
        ),
        notification(json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"Read"})),
        notification(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","kind":"read","locations":[{"path":"README.md"}],"rawInput":{"path":"README.md"}}),
        ),
        notification(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{"type":"text","text":"read"}]}),
        ),
    ]);
    assert_eq!(history.len(), 3);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(
        history[0].pointer("/content/0/text"),
        Some(&json!("hello world"))
    );
    assert_eq!(
        history[1].pointer("/content/0/type"),
        Some(&json!("toolCall"))
    );
    assert_eq!(history[1].pointer("/content/0/name"), Some(&json!("Read")));
    assert_eq!(
        history[1].pointer("/content/0/arguments/path"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        history[1].pointer("/content/0/toolMetadata/category"),
        Some(&json!("read"))
    );
    assert_eq!(
        history[1].pointer("/content/0/toolMetadata/targets/0"),
        Some(&json!("README.md"))
    );
    assert_eq!(
        history[1].pointer("/content/0/toolMetadata/native/title"),
        Some(&json!("Read"))
    );
    assert_eq!(history[2]["role"], "toolResult");
}

#[test]
fn replay_retains_output_emitted_before_completion() {
    let notification = |update| AcpInbound::Notification {
        method: "session/update".into(),
        params: json!({"update":update}),
    };
    let history = replay_history([
        notification(json!({
            "sessionUpdate":"tool_call",
            "toolCallId":"partial",
            "title":"Long task"
        })),
        notification(json!({
            "sessionUpdate":"tool_call_update",
            "toolCallId":"partial",
            "content":[{"type":"text","text":"output before completion"}]
        })),
        notification(json!({
            "sessionUpdate":"tool_call_update",
            "toolCallId":"partial",
            "status":"completed"
        })),
    ]);
    assert_eq!(
        history[1].pointer("/content/0/text"),
        Some(&json!("output before completion"))
    );
}

#[test]
fn replay_cursor_edits_expose_canonical_args_and_diff_details() {
    let notification = |update| AcpInbound::Notification {
        method: "session/update".into(),
        params: json!({"update":update}),
    };
    let history = replay_history([
        notification(json!({
            "sessionUpdate":"tool_call",
            "toolCallId":"edit-1",
            "kind":"edit",
            "title":"Edit src/main.rs",
            "locations":[{"path":"src/main.rs","line":4}],
            "rawInput":{
                "filePath":"src/main.rs",
                "old_string":"old",
                "new_string":"new"
            }
        })),
        notification(json!({
            "sessionUpdate":"tool_call_update",
            "toolCallId":"edit-1",
            "status":"completed",
            "content":[{
                "type":"diff",
                "path":"src/main.rs",
                "oldText":"old",
                "newText":"new"
            }]
        })),
    ]);
    assert_eq!(history[0].pointer("/content/0/name"), Some(&json!("edit")));
    assert_eq!(
        history[0].pointer("/content/0/arguments"),
        Some(&json!({
            "path":"src/main.rs",
            "oldText":"old",
            "newText":"new"
        }))
    );
    assert_eq!(history[1]["role"], "toolResult");
    assert_eq!(
        history[1].pointer("/details/diff"),
        Some(&json!("-old\n+new\n"))
    );
    assert_eq!(
        history[1].pointer("/details/firstChangedLine"),
        Some(&json!(4))
    );
}

#[test]
fn replay_handles_completed_tool_as_its_first_update() {
    let history = replay_history([AcpInbound::Notification {
        method: "session/update".into(),
        params: json!({"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"done",
            "kind":"fetch",
            "title":"Fetch docs",
            "status":"completed",
            "rawInput":{"url":"https://example.com"},
            "content":[{"type":"text","text":"done"}]
        }}),
    }]);
    assert_eq!(history.len(), 2);
    assert_eq!(
        history[0].pointer("/content/0/name"),
        Some(&json!("web_fetch"))
    );
    assert_eq!(
        history[0].pointer("/content/0/toolMetadata/category"),
        Some(&json!("fetch"))
    );
    assert_eq!(history[1]["role"], "toolResult");
}

#[test]
fn catalog_reuse_requires_the_same_live_project() {
    let project = PathBuf::from("/project");
    assert!(catalog_is_reusable(&project, &project, true));
    assert!(!catalog_is_reusable(&project, Path::new("/other"), true));
    assert!(!catalog_is_reusable(&project, &project, false));
}

#[test]
fn replay_ignores_updates_from_other_sessions() {
    let history = replay_history_for_session(
        [
            AcpInbound::Notification {
                method: "session/update".into(),
                params: json!({
                    "sessionId": "other",
                    "update": {
                        "sessionUpdate": "user_message_chunk",
                        "content": {"type": "text", "text": "stale"}
                    }
                }),
            },
            AcpInbound::Notification {
                method: "session/update".into(),
                params: json!({
                    "sessionId": "wanted",
                    "update": {
                        "sessionUpdate": "user_message_chunk",
                        "content": {"type": "text", "text": "kept"}
                    }
                }),
            },
        ],
        Some("wanted"),
    );
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].pointer("/content/0/text"), Some(&json!("kept")));
}

#[test]
fn resume_history_uses_the_session_load_replay() {
    let profile = AcpProfile {
        backend: "cursor-cli",
        name: "Cursor",
        command: "agent",
        path_environment: "FARCASTER_CURSOR_PATH",
        arguments: &["acp"],
        auth_method: Some("cursor_login"),
        force_argument: Some("--force"),
    };
    let history = discovered_history(
        &profile,
        [AcpInbound::Notification {
            method: "session/update".into(),
            params: json!({
                "sessionId": "cursor-historical",
                "update": {
                    "sessionUpdate": "user_message_chunk",
                    "content": {"type": "text", "text": "replayed"}
                }
            }),
        }],
        &json!({
            "configOptions": [
                {"id": "model", "category": "model", "currentValue": "composer-2"},
                {"id": "thinking", "category": "thought_level", "currentValue": "high"}
            ]
        }),
        "cursor-historical",
    );
    assert_eq!(
        history.messages[0].pointer("/content/0/text"),
        Some(&json!("replayed"))
    );
    assert_eq!(
        history
            .model
            .as_ref()
            .map(|(provider, id)| (provider.as_str(), id.as_str())),
        Some(("cursor-cli", "composer-2"))
    );
    assert_eq!(history.thinking_level.as_deref(), Some("high"));
}
