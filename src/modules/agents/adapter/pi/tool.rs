use serde_json::Value;

use crate::agents::{ToolCategory, ToolMetadata};

pub(super) fn annotate_pi_value(value: &mut Value) {
    if value.get("type").and_then(Value::as_str) == Some("tool_execution_start") {
        annotate_tool(value, "toolName", "args");
    }
    if let Some(message) = value.get_mut("message") {
        annotate_pi_message(message);
    }
    if let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            annotate_pi_message(message);
        }
    }
    if let Some(tool_call) = value.pointer_mut("/assistantMessageEvent/toolCall") {
        annotate_tool_call(tool_call);
    }
    if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
        for entry in entries {
            if let Some(message) = entry.get_mut("message") {
                annotate_pi_message(message);
            }
        }
    }
}

pub(crate) fn annotate_pi_message(message: &mut Value) {
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for block in content {
        annotate_tool_call(block);
    }
}

fn annotate_tool_call(block: &mut Value) {
    if block.get("type").and_then(Value::as_str) == Some("toolCall") {
        annotate_tool(block, "name", "arguments");
    }
}

fn annotate_tool(value: &mut Value, name_field: &str, args_field: &str) {
    let Some(name) = value.get(name_field).and_then(Value::as_str) else {
        return;
    };
    let args = value.get(args_field).cloned().unwrap_or(Value::Null);
    let native = value
        .get("toolMetadata")
        .and_then(|metadata| metadata.get("native"))
        .cloned()
        .unwrap_or_else(|| args.clone());
    let metadata = pi_tool_metadata(name, &args, native);
    value["toolMetadata"] = serde_json::to_value(metadata).expect("tool metadata serializes");
}

fn pi_tool_metadata(name: &str, args: &Value, native: Value) -> ToolMetadata {
    let (category, verb, target_keys): (ToolCategory, Option<&str>, &[&str]) = match name {
        "read" => (ToolCategory::Read, Some("Read"), &["path"]),
        "grep" | "find" => (ToolCategory::Search, Some("Search"), &["path", "directory"]),
        "ls" => (ToolCategory::List, Some("List"), &["path"]),
        "write" => (ToolCategory::Change, Some("Write"), &["path"]),
        "edit" => (ToolCategory::Change, Some("Edit"), &["path"]),
        "bash" => (ToolCategory::Execute, Some("Run command"), &[]),
        "web_search" => (ToolCategory::Fetch, Some("Search web"), &["url"]),
        "web_fetch" | "fetch" => (ToolCategory::Fetch, Some("Fetch"), &["url"]),
        "worker_start" => (ToolCategory::Delegate, Some("Start worker"), &[]),
        "worker_send" => (ToolCategory::Delegate, Some("Message worker"), &[]),
        "worker_wait" => (ToolCategory::Delegate, Some("Wait for worker"), &[]),
        "worker_list" => (ToolCategory::Delegate, Some("List workers"), &[]),
        _ => (ToolCategory::Other, None, &[]),
    };
    let targets = string_targets(args, target_keys);
    let title = verb.map(|verb| match targets.first() {
        Some(target) => format!("{verb} {target}"),
        None => verb.to_owned(),
    });
    ToolMetadata {
        category: Some(category),
        title,
        targets,
        native: Some(native),
    }
}

fn string_targets(args: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| args.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builtin_metadata_matches_across_live_history_and_stream_events() {
        let arguments = json!({"path": "src/lib.rs", "offset": 2});
        let mut live = json!({
            "type": "tool_execution_start",
            "toolCallId": "read-1",
            "toolName": "read",
            "args": arguments.clone()
        });
        let mut history = json!({
            "role": "assistant",
            "content": [{
                "type": "toolCall",
                "id": "read-1",
                "name": "read",
                "arguments": arguments.clone()
            }]
        });
        let mut stream = json!({
            "type": "message_update",
            "assistantMessageEvent": {
                "type": "toolcall_end",
                "toolCall": {
                    "type": "toolCall",
                    "id": "read-1",
                    "name": "read",
                    "arguments": arguments.clone()
                }
            }
        });

        annotate_pi_value(&mut live);
        annotate_pi_message(&mut history);
        annotate_pi_value(&mut stream);

        assert_eq!(live["args"], arguments);
        let expected = live["toolMetadata"].clone();
        assert_eq!(expected["category"], "read");
        assert_eq!(expected["targets"], json!(["src/lib.rs"]));
        assert_eq!(history["content"][0]["toolMetadata"], expected);
        assert_eq!(
            stream["assistantMessageEvent"]["toolCall"]["toolMetadata"],
            expected
        );
    }

    #[test]
    fn leaves_custom_tool_intent_unknown_and_keeps_native_metadata() {
        let mut message = json!({
            "role": "assistant",
            "content": [{
                "type": "toolCall",
                "id": "custom-1",
                "name": "mcp_custom_action",
                "arguments": {"path": "do-not-guess"},
                "toolMetadata": {"native": {"provider": "custom"}}
            }]
        });
        annotate_pi_message(&mut message);
        let metadata = &message["content"][0]["toolMetadata"];
        assert_eq!(metadata["category"], "other");
        assert!(metadata["title"].is_null());
        assert!(metadata.get("targets").is_none());
        assert_eq!(metadata["native"], json!({"provider": "custom"}));
    }
}
