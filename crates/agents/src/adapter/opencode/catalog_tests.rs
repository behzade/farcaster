use super::*;
use crate::Backend;

#[test]
fn restores_the_latest_opencode_session_identity() {
    let messages = vec![
        json!({
            "type": "assistant",
            "model": {"id": "old", "providerID": "provider"}
        }),
        json!({
            "type": "model-switched",
            "model": {"id": "latest", "providerID": "provider", "variant": "high"}
        }),
    ];

    let historical = latest_identity(&messages, None).expect("message identity");
    assert_eq!(historical.id, "latest");
    assert_eq!(historical.variant.as_deref(), Some("high"));

    let saved = OpenCodeModelSelection {
        id: "saved".into(),
        provider_id: "provider".into(),
        variant: Some("max".into()),
    };
    assert_eq!(latest_identity(&messages, Some(&saved)), Some(saved));
}

#[test]
fn default_variant_is_unset_without_resurrecting_an_older_effort() {
    let old = json!({"model": {"id": "astra", "providerID": "openai", "variant": "high"}});
    for selection in [
        json!({"id": "astra", "providerID": "openai"}),
        json!({"id": "astra", "providerID": "openai", "variant": "default"}),
    ] {
        let saved: OpenCodeModelSelection =
            serde_json::from_value(selection.clone()).expect("decode saved selection");
        assert_eq!(
            latest_identity(std::slice::from_ref(&old), Some(&saved))
                .expect("saved identity")
                .variant,
            None
        );
        assert_eq!(
            latest_identity(&[old.clone(), json!({"model": selection})], None)
                .expect("latest identity")
                .variant,
            None
        );
    }
    let none: OpenCodeModelSelection = serde_json::from_value(json!({
        "id": "glm", "providerID": "provider", "variant": "none"
    }))
    .expect("decode model selection");
    assert_eq!(none.variant.as_deref(), Some("none"));
}

#[test]
fn translates_session_metadata() -> Result<(), String> {
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let value = json!({
        "id": "session-1",
        "parentID": "parent-1",
        "location": {"directory": project},
        "title": "Implement feature",
        "time": {"created": 1, "updated": 2},
        "tokens": {"input": 100, "output": 20, "reasoning": 5, "cache": {"read": 80, "write": 10}},
    });
    let session = summary(project.as_path(), &value).ok_or("summary")??;
    assert_eq!(session.harness, Backend::OpenCode);
    assert_eq!(session.parent_session.as_deref(), Some("parent-1"));
    assert_eq!(session.title, "Implement feature");
    assert_eq!(session.usage.input, 100);
    assert_eq!(session.usage.output, 25);
    assert_eq!(session.usage.cache_read, 80);
    Ok(())
}

#[test]
fn restores_delivery_receipts_from_before_the_backend_rename() {
    let messages = history_messages(&json!({
        "id": "msg_opencode2-request-1",
        "type": "user",
        "text": "saved prompt",
    }));
    assert_eq!(messages[0]["submissionId"], "opencode2-request-1");
    assert_eq!(messages[0]["deliveryStatus"], "delivered");
}

#[test]
fn delivery_reconciliation_distinguishes_history_from_inbox_by_exact_id() {
    let evidence = prompt_delivery_reconciliation(
        &[
            json!({"id":"msg_opencode-delivered", "type":"user"}),
            json!({
                "id":"msg_farcaster_retry",
                "type":"user",
                "metadata":{"farcasterSubmissionId":"opencode-retried-delivery"}
            }),
            json!({"id":"msg_unrelated", "type":"user"}),
        ],
        &[
            json!({"id":"msg_opencode-pending", "type":"user"}),
            json!({
                "id":"msg_farcaster_retry",
                "type":"user",
                "payload":{"metadata":{"farcasterSubmissionId":"opencode-retried-pending"}}
            }),
            json!({"id":"msg_other", "type":"synthetic"}),
        ],
    );
    assert_eq!(
        evidence.delivered,
        ["opencode-delivered", "opencode-retried-delivery"]
    );
    assert_eq!(
        evidence.pending,
        ["opencode-pending", "opencode-retried-pending"]
    );
}

#[test]
fn preserves_base64_and_data_uri_images_in_user_history() {
    let messages = history_messages(&json!({
        "id": "msg_opencode-request-1",
        "type": "user",
        "text": "compare",
        "files": [
            {
                "mime": "image/png",
                "source": {"type": "base64", "data": "AQID"}
            },
            {"uri": "data:image/jpeg;base64,BAUG"}
        ]
    }));

    assert_eq!(
        messages[0]["content"],
        json!([
            {"type": "text", "text": "compare"},
            {"type": "image", "mimeType": "image/png", "data": "AQID"},
            {"type": "image", "mimeType": "image/jpeg", "data": "BAUG"},
        ])
    );
    assert_eq!(messages[0]["submissionId"], "opencode-request-1");
    assert_eq!(messages[0]["deliveryStatus"], "delivered");
}

#[test]
fn translates_ordered_reasoning_tool_results_and_text() {
    let messages = history_messages(&json!({
        "type": "assistant",
        "content": [
            {"type": "reasoning", "text": "Inspect the file"},
            {
                "type": "tool",
                "id": "tool-1",
                "name": "read_file",
                "state": {
                    "status": "completed",
                    "input": {"filePath": "src/main.rs"},
                    "content": [{"type": "text", "text": "fn main() {}"}]
                }
            },
            {"type": "text", "text": "Done"}
        ],
        "tokens": {"input": 10, "output": 2, "cache": {"read": 8, "write": 0}},
    }));

    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].pointer("/content/0/type"),
        Some(&json!("thinking"))
    );
    assert_eq!(
        messages[0].pointer("/content/1/type"),
        Some(&json!("toolCall"))
    );
    assert_eq!(messages[0].pointer("/content/1/name"), Some(&json!("read")));
    assert_eq!(
        messages[0].pointer("/content/1/arguments/path"),
        Some(&json!("src/main.rs"))
    );
    assert_eq!(messages[0].pointer("/content/2/text"), Some(&json!("Done")));
    assert_eq!(
        messages[0].pointer("/content/1/toolMetadata/category"),
        Some(&json!("read"))
    );
    assert_eq!(
        messages[0].pointer("/content/1/toolMetadata/targets/0"),
        Some(&json!("src/main.rs"))
    );
    assert_eq!(
        messages[0].pointer("/content/1/toolMetadata/native/state/input/filePath"),
        Some(&json!("src/main.rs"))
    );
    assert_eq!(messages[0].pointer("/usage/input"), Some(&json!(10)));
    assert_eq!(messages[1]["role"], "toolResult");
    assert_eq!(
        messages[1].pointer("/content/0/text"),
        Some(&json!("fn main() {}"))
    );
    assert_eq!(messages[1]["isError"], false);
}

#[test]
fn translates_failed_tool_results() {
    let messages = history_messages(&json!({
        "type": "assistant",
        "content": [{
            "type": "tool",
            "id": "tool-1",
            "name": "shell",
            "state": {
                "status": "error",
                "input": {"cmd": "false"},
                "error": {"type": "Unknown", "message": "command failed"}
            }
        }]
    }));

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].pointer("/content/0/name"), Some(&json!("bash")));
    assert_eq!(
        messages[0].pointer("/content/0/arguments/command"),
        Some(&json!("false"))
    );
    assert_eq!(
        messages[1].pointer("/content/0/text"),
        Some(&json!("command failed"))
    );
    assert_eq!(messages[1]["isError"], true);
}

#[test]
fn interrupted_subagent_history_does_not_keep_running_metadata() {
    let messages = history_messages(&json!({
        "type": "assistant",
        "content": [{
            "type": "tool",
            "id": "subagent-1",
            "name": "subagent",
            "state": {
                "status": "error",
                "input": {"prompt": "Inspect the implementation"},
                "metadata": {"status": "running", "sessionID": "child-1"},
                "error": {
                    "type": "aborted",
                    "message": "Tool execution interrupted (sessionID: child-1)"
                }
            }
        }]
    }));

    assert_eq!(
        messages[0].pointer("/content/0/toolMetadata/native/state/metadata/status"),
        Some(&json!("interrupted"))
    );
    assert_eq!(messages[1]["isError"], true);
}

#[test]
fn restored_patch_has_a_file_edit_presentation() {
    let messages = history_messages(&json!({
        "type": "assistant",
        "content": [{
            "type": "tool", "id": "patch-1", "name": "patch",
            "state": {
                "status": "completed",
                "input": {"patchText": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch"},
                "content": [{"type": "text", "text": "Success"}],
                "metadata": {"files": [{
                    "file": "src/main.rs",
                    "patch": "@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
                    "status": "modified", "additions": 2, "deletions": 1
                }]}
            }
        }]
    }));
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.replace_history(&messages);
    let item = &conversation.items[0];
    let presentation = item.tool_presentation.as_ref().expect("file edit row");
    assert_eq!(presentation.path(), "src/main.rs");
    assert_eq!(presentation.counts(), (2, 1));
    assert_eq!(
        item.tool_details
            .as_ref()
            .expect("tool details")
            .metadata
            .targets,
        ["src/main.rs"]
    );
    assert!(!item.is_error);
    assert!(!item.streaming);
}
