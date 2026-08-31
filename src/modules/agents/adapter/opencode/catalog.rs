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
    let mut child = Command::new(program)
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
        usage: DiscoveredUsage::default(),
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
    Some(json!({"role": role, "content": content}))
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
        });
        let session = summary(project.as_path(), &value).ok_or("summary")??;
        assert_eq!(session.harness, "opencode2");
        assert_eq!(session.title, "Implement feature");
        Ok(())
    }
}
