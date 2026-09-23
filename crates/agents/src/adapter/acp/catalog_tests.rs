use crate::Backend;
use std::path::Path;

use super::*;
use crate::adapter::acp::events::AcpInbound;

#[cfg(unix)]
#[test]
fn failed_cursor_catalog_read_closes_and_cleans_up_its_empty_session() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    const SCRIPT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) result='{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"close":{}}},"authMethods":[]}' ;;
    *'"method":"session/new"'*) result='{"sessionId":"catalog-only","configOptions":[]}' ;;
    *'"method":"cursor/list_available_models"'*) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"models unavailable"}}\n' "$id"; continue ;;
    *'"method":"session/close"'*) result='{}' ;;
    *) exit 2 ;;
  esac
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#;
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = project.path().join("fake-cursor");
    std::fs::write(&executable, SCRIPT).map_err(|error| error.to_string())?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let mut profile = super::super::super::cursor::PROFILE;
    profile.command = Box::leak(executable.to_string_lossy().into_owned().into_boxed_str());
    profile.path_environment = "FARCASTER_UNUSED_CATALOG_TEST_PATH";
    let (sender, receiver) = mpsc::channel();
    let result = load_configuration_with_cleanup(&profile, project.path(), move |id| {
        let _ = sender.send(id.to_owned());
    });
    assert!(result.is_err());
    assert_eq!(
        receiver.recv().map_err(|error| error.to_string())?,
        "catalog-only"
    );
    let requests = std::fs::read_to_string(executable.with_extension("requests"))
        .map_err(|error| error.to_string())?;
    assert!(requests.contains("\"method\":\"session/close\""));
    Ok(())
}

#[test]
fn catalog_exchange_times_out_when_agent_stalls() {
    let (release, stalled) = mpsc::channel::<()>();
    let error = run_with_timeout(Duration::from_millis(20), move || {
        let _ = stalled.recv();
        Ok(())
    })
    .expect_err("invalid test input must fail");
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

fn catalog_launch(project: &Path, account: &str) -> CatalogLaunch {
    let mut command = Command::new("/agent");
    command
        .arg("acp")
        .env("AGENT_ACCOUNT", account)
        .env("PWD", project)
        .current_dir(project);
    CatalogLaunch::from_command(&command)
}

#[test]
fn catalog_reuse_follows_launch_context_not_request_cwd() {
    let existing = catalog_launch(Path::new("/project"), "personal");
    let requested = catalog_launch(Path::new("/other"), "personal");
    assert!(catalog_is_reusable(&existing, &requested, true));
    assert!(!catalog_is_reusable(&existing, &requested, false));
    assert!(!catalog_is_reusable(
        &existing,
        &catalog_launch(Path::new("/other"), "work"),
        true
    ));
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
        backend: Backend::Cursor,
        name: "Cursor",
        command: "agent",
        path_environment: "FARCASTER_CURSOR_PATH",
        arguments: &["acp"],
        auth_method: None,
        force_argument: Some("--force"),
        resume_method: "session/load",
        permission_modes: None,
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
