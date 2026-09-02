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

fn with_server<T>(
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
        search,
    }))
}

fn history_messages(value: &Value) -> Vec<Value> {
    let role = value.get("role").and_then(Value::as_str).or_else(|| {
        match value.get("type").and_then(Value::as_str)? {
            "user" => Some("user"),
            "assistant" => Some("assistant"),
            _ => None,
        }
    });
    match role {
        Some("user") => vec![json!({
            "role": "user",
            "content": value
                .get("text")
                .and_then(Value::as_str)
                .map(|text| vec![json!({"type": "text", "text": text})])
                .or_else(|| value.get("content").and_then(Value::as_array).cloned())
                .unwrap_or_default(),
        })],
        Some("assistant") => assistant_history_messages(value),
        Some(_) if value.get("role").is_some() => vec![value.clone()],
        _ => Vec::new(),
    }
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
            Some("text" | "thinking" | "toolCall") => content.push(block.clone()),
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
                content.push(json!({
                    "type": "toolCall",
                    "id": id,
                    "name": name,
                    "arguments": arguments,
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
mod tests {
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
}
