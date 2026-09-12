use super::*;

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
    assert_eq!(session.harness, "opencode2");
    assert_eq!(session.parent_session.as_deref(), Some("parent-1"));
    assert_eq!(session.title, "Implement feature");
    assert_eq!(session.usage.input, 100);
    assert_eq!(session.usage.output, 25);
    assert_eq!(session.usage.cache_read, 80);
    Ok(())
}

#[test]
fn preserves_base64_and_data_uri_images_in_user_history() {
    let messages = history_messages(&json!({
        "id": "msg_opencode2-request-1",
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
    assert_eq!(messages[0]["submissionId"], "opencode2-request-1");
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
