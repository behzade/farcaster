use serde_json::Value;

mod edit_diff;

use crate::agents::{ToolCategory, ToolMetadata};

pub(super) fn annotate_pi_value(value: &mut Value) {
    if value.get("type").and_then(Value::as_str) == Some("tool_execution_end")
        && let Some(result) = value.get_mut("result")
    {
        annotate_edit_result(result);
    }
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
    if message.get("role").and_then(Value::as_str) == Some("toolResult") {
        annotate_edit_result(message);
    }
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

fn annotate_edit_result(result: &mut Value) {
    if let Some(details) = result.get_mut("details")
        && let Some(diff) = details.get("diff").and_then(Value::as_str)
        && let Some(unified) = edit_diff::unified_diff(diff)
    {
        details["unifiedDiff"] = Value::String(unified);
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
#[path = "tool_tests.rs"]
mod tests;
