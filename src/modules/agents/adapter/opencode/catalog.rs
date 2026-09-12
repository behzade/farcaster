use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::{contract::OpenCodeModelSelection, server::OpenCodeServerProcess};
use crate::agents::{DiscoveredHistory, DiscoveredSession, DiscoveredUsage};

use super::super::{
    child_stderr,
    main_session::{external_session_locator, external_session_path},
};

pub(in crate::modules::agents::adapter) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    with_server(|server| {
        let value = server.client().list_sessions(query)?;
        let rows = value
            .as_array()
            .or_else(|| value.get("data").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();
        rows.iter()
            .filter_map(|value| summary(locator_root, value))
            .collect()
    })
}

pub(in crate::modules::agents::adapter) fn rename_session(
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    with_server(|server| server.client().rename_session(session_id, name))
}

pub(in crate::modules::agents::adapter) fn delete_session(session_id: &str) -> Result<(), String> {
    with_server(|server| server.client().delete_session(session_id))
}

pub(in crate::modules::agents::adapter) fn load_history(
    path: &Path,
) -> Result<DiscoveredHistory, String> {
    let locator = external_session_locator("opencode2", path)
        .ok_or_else(|| format!("invalid OpenCode session locator: {}", path.display()))?;
    with_server(|server| {
        let response = server.client().session_messages(&locator)?;
        let rows = response
            .as_array()
            .or_else(|| response.get("data").and_then(Value::as_array))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let session = server.client().get_session(&locator)?;
        let identity = latest_identity(rows, session.model.as_ref());
        let messages = rows.iter().flat_map(history_messages).collect();
        let (model, thinking_level) = identity.map_or((None, None), |identity| {
            (Some((identity.provider_id, identity.id)), identity.variant)
        });
        Ok(DiscoveredHistory {
            messages,
            model,
            thinking_level,
        })
    })
}

fn latest_identity(
    messages: &[Value],
    session: Option<&OpenCodeModelSelection>,
) -> Option<OpenCodeModelSelection> {
    session.cloned().or_else(|| {
        messages
            .iter()
            .rev()
            .find_map(|message| serde_json::from_value(message.get("model")?.clone()).ok())
    })
}

pub(super) fn with_server<T>(
    operation: impl FnOnce(&OpenCodeServerProcess) -> Result<T, String>,
) -> Result<T, String> {
    let program = std::env::var_os("FARCASTER_OPENCODE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "opencode2".into());
    let password = format!("farcaster-catalog-{}", std::process::id());
    let mut command = Command::new(program);
    let mut child = command
        .args(["serve", "--stdio", "--print-logs"])
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start OpenCode catalog server: {error}"))?;
    child_stderr::capture(&mut child, "opencode-catalog")?;
    let mut server = OpenCodeServerProcess::attach(child, "opencode", password)?;
    let result = operation(&server);
    let close = server.terminate();
    match (result, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn summary(locator_root: &Path, value: &Value) -> Option<Result<DiscoveredSession, String>> {
    let id = value.get("id")?.as_str()?;
    let directory = value
        .pointer("/location/directory")
        .and_then(Value::as_str)?;
    let project = PathBuf::from(directory);
    if !project.is_dir() || crate::projects::is_temporary_project(&project) {
        return None;
    }
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("New OpenCode session")
        .to_owned();
    let first_user_message = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let modified = millis(value.pointer("/time/updated").and_then(Value::as_u64));
    let timestamp = value
        .pointer("/time/created")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let archived = value
        .get("archived")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .pointer("/time/archived")
            .is_some_and(|value| !value.is_null());
    let is_running = value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "running" | "active"));
    let path = external_session_path(locator_root, "opencode2", id);
    let search = format!("{title} {first_user_message} {directory} opencode");
    Some(Ok(DiscoveredSession {
        id: id.to_owned(),
        harness: "opencode2".into(),
        path,
        project,
        title,
        first_user_message,
        timestamp,
        parent_session: value
            .get("parentID")
            .and_then(Value::as_str)
            .map(str::to_owned),
        modified,
        message_count: value
            .get("messageCount")
            .and_then(Value::as_u64)
            .and_then(|count| count.try_into().ok())
            .unwrap_or(0),
        usage: opencode_usage(value),
        archived,
        is_running,
        model: None,
        thinking_level: None,
        search,
    }))
}

pub(super) fn history_messages(value: &Value) -> Vec<Value> {
    let role = value.get("role").and_then(Value::as_str).or_else(|| {
        match value.get("type").and_then(Value::as_str)? {
            "user" => Some("user"),
            "assistant" => Some("assistant"),
            _ => None,
        }
    });
    match role {
        Some("user") => {
            let mut message = json!({
                "role": "user",
                "content": opencode_user_content(value),
            });
            if let Some(submission_id) = value
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| id.strip_prefix("msg_"))
                .filter(|id| id.starts_with("opencode2-"))
            {
                message["submissionId"] = Value::String(submission_id.to_owned());
                message["deliveryStatus"] = Value::String("delivered".into());
            }
            vec![message]
        }
        Some("assistant") => assistant_history_messages(value),
        Some(_) if value.get("role").is_some() => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn opencode_user_content(value: &Value) -> Vec<Value> {
    let mut content = value
        .get("text")
        .and_then(Value::as_str)
        .map(|text| vec![json!({"type": "text", "text": text})])
        .or_else(|| value.get("content").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    for file in value
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(image) = opencode_image_content(file)
            && !content.contains(&image)
        {
            content.push(image);
        }
    }
    content
}

fn opencode_image_content(file: &Value) -> Option<Value> {
    let mime = file
        .get("mime")
        .or_else(|| file.get("mimeType"))
        .and_then(Value::as_str);
    let data = file
        .pointer("/source/data")
        .or_else(|| file.get("data"))
        .and_then(Value::as_str);
    if let (Some(mime), Some(data)) = (mime, data)
        && mime.starts_with("image/")
    {
        return Some(json!({"type": "image", "mimeType": mime, "data": data}));
    }
    let uri = file
        .pointer("/source/uri")
        .or_else(|| file.get("uri"))
        .and_then(Value::as_str)?;
    let encoded = uri.strip_prefix("data:")?;
    let (mime, data) = encoded.split_once(";base64,")?;
    mime.starts_with("image/")
        .then(|| json!({"type": "image", "mimeType": mime, "data": data}))
}

fn assistant_history_messages(value: &Value) -> Vec<Value> {
    let mut content = Vec::new();
    let mut results = Vec::new();
    for block in value
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text" | "thinking") => content.push(block.clone()),
            Some("toolCall") => {
                let mut tool_call = block.clone();
                let reported_name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let (name, arguments) = super::tool::normalize_opencode_tool(
                    reported_name,
                    block.get("arguments").unwrap_or(&Value::Null),
                );
                let metadata = opencode_history_tool_metadata(block, &name, &arguments);
                tool_call["name"] = Value::String(name);
                tool_call["arguments"] = arguments;
                tool_call["toolMetadata"] =
                    serde_json::to_value(metadata).expect("tool metadata serializes");
                content.push(tool_call);
            }
            Some("reasoning") => content.push(json!({
                "type": "thinking",
                "thinking": block.get("text").and_then(Value::as_str).unwrap_or_default(),
            })),
            Some("tool") => {
                let Some(id) = block.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let state = block.get("state").unwrap_or(&Value::Null);
                let reported_name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let (name, arguments) = super::tool::normalize_opencode_tool(
                    reported_name,
                    state.get("input").unwrap_or(&Value::Null),
                );
                let is_error = opencode_tool_failed(state);
                let metadata = opencode_history_tool_metadata(block, &name, &arguments);
                content.push(json!({
                    "type": "toolCall",
                    "id": id,
                    "name": name,
                    "arguments": arguments,
                    "toolMetadata": metadata,
                }));
                results.push(json!({
                    "role": "toolResult",
                    "toolCallId": id,
                    "toolName": name,
                    "content": opencode_tool_result_content(state, is_error),
                    "isError": is_error,
                }));
            }
            _ => {}
        }
    }
    if content.is_empty()
        && let Some(text) = value.get("text").and_then(Value::as_str)
    {
        content.push(json!({"type": "text", "text": text}));
    }
    let usage = opencode_usage(value);
    let mut messages = vec![json!({
        "role": "assistant",
        "content": content,
        "usage": {
            "input": usage.input,
            "output": usage.output,
            "cacheRead": usage.cache_read,
            "cacheWrite": usage.cache_write,
            "totalTokens": usage.total,
        },
    })];
    messages.append(&mut results);
    messages
}

fn opencode_history_tool_metadata(
    block: &Value,
    name: &str,
    arguments: &Value,
) -> crate::agents::ToolMetadata {
    let native = block
        .get("toolMetadata")
        .and_then(|metadata| metadata.get("native"))
        .cloned()
        .unwrap_or_else(|| block.clone());
    super::tool::opencode_tool_metadata(name, arguments, native)
}

fn opencode_tool_failed(state: &Value) -> bool {
    state
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "error" | "failed"))
}

fn opencode_tool_result_content(state: &Value, is_error: bool) -> Vec<Value> {
    if is_error {
        return state
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| state.get("error").and_then(Value::as_str))
            .map(|text| vec![json!({"type": "text", "text": text})])
            .unwrap_or_default();
    }
    if let Some(content) = state.get("content").and_then(Value::as_array) {
        return content.clone();
    }
    state
        .get("output")
        .map(|output| {
            let text = output
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| output.to_string());
            vec![json!({"type": "text", "text": text})]
        })
        .unwrap_or_default()
}

fn opencode_usage(value: &Value) -> DiscoveredUsage {
    let tokens = value.get("tokens").unwrap_or(value);
    let input = token(tokens, "input", "tokens_input");
    let output = token(tokens, "output", "tokens_output").saturating_add(token(
        tokens,
        "reasoning",
        "tokens_reasoning",
    ));
    let cache = tokens.get("cache").unwrap_or(tokens);
    let cache_read = token(cache, "read", "tokens_cache_read");
    let cache_write = token(cache, "write", "tokens_cache_write");
    DiscoveredUsage {
        input,
        output,
        cache_read,
        cache_write,
        total: input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write),
        cost_micros: value
            .get("cost")
            .and_then(Value::as_f64)
            .map(|cost| (cost * 1_000_000.0).max(0.0) as u64)
            .unwrap_or(0),
    }
}

fn token(value: &Value, nested: &str, flat: &str) -> u64 {
    value
        .get(nested)
        .and_then(Value::as_u64)
        .or_else(|| value.get(flat).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn millis(value: Option<u64>) -> SystemTime {
    value.map_or_else(SystemTime::now, |value| {
        UNIX_EPOCH + Duration::from_millis(value)
    })
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
