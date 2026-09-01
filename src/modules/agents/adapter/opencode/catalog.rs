use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::server::OpenCodeServerProcess;
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
        let messages = server.client().session_messages(&locator)?;
        let messages = messages
            .as_array()
            .or_else(|| messages.get("data").and_then(Value::as_array))
            .into_iter()
            .flatten()
            .filter_map(history_message)
            .map(|message| json!({"type": "message", "message": message}))
            .collect();
        let _session = server.client().get_session(&locator)?;
        Ok(DiscoveredHistory {
            messages,
            model: None,
            thinking_level: None,
        })
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
    command.args(["serve", "--stdio", "--print-logs"]);
    super::configure_permissions(&mut command);
    let mut child = command
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

fn summary(
    locator_root: &Path,
    value: &Value,
) -> Option<Result<DiscoveredSession, String>> {
    let id = value.get("id")?.as_str()?;
    let directory = value
        .pointer("/location/directory")
        .and_then(Value::as_str)?;
    let project = PathBuf::from(directory);
    if !project.is_dir() {
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
        || value.pointer("/time/archived").is_some_and(|value| !value.is_null());
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

fn history_message(value: &Value) -> Option<Value> {
    let role = value.get("role").and_then(Value::as_str).or_else(|| {
        match value.get("type").and_then(Value::as_str)? {
            "user" => Some("user"),
            "assistant" => Some("assistant"),
            _ => None,
        }
    })?;
    let content = if let Some(content) = value.get("content").and_then(Value::as_array) {
        content.clone()
    } else if let Some(text) = value.get("text").and_then(Value::as_str) {
        vec![json!({"type": "text", "text": text})]
    } else {
        Vec::new()
    };
    let mut message = json!({"role": role, "content": content});
    if role == "assistant" {
        let usage = opencode_usage(value);
        message["usage"] = json!({
            "input": usage.input,
            "output": usage.output,
            "cacheRead": usage.cache_read,
            "cacheWrite": usage.cache_write,
            "totalTokens": usage.total,
        });
    }
    Some(message)
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
    fn translates_session_metadata() -> Result<(), String> {
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let value = json!({
            "id": "session-1",
            "location": {"directory": project},
            "title": "Implement feature",
            "time": {"created": 1, "updated": 2},
            "tokens": {"input": 100, "output": 20, "reasoning": 5, "cache": {"read": 80, "write": 10}},
        });
        let session = summary(project.as_path(), &value).ok_or("summary")??;
        assert_eq!(session.harness, "opencode2");
        assert_eq!(session.title, "Implement feature");
        assert_eq!(session.usage.input, 100);
        assert_eq!(session.usage.output, 25);
        assert_eq!(session.usage.cache_read, 80);
        Ok(())
    }
}
