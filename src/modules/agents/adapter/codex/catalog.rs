use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::{connection::CodexConnection, contract::CodexClientInfo};
use crate::agents::{DiscoveredHistory, DiscoveredSession, DiscoveredUsage};

use super::super::{
    child_stderr,
    main_session::{external_session_locator, external_session_path},
};

pub(in crate::modules::agents::adapter) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    with_connection(|connection| {
        let mut sessions = Vec::new();
        for archived in [false, true] {
            let id = connection.send_request(
                "thread/list",
                json!({
                    "archived": archived,
                    "limit": 100,
                    "searchTerm": (!query.is_empty()).then_some(query),
                    "sortKey": "updated_at",
                    "sortDirection": "desc",
                }),
            )?;
            let response: Value = connection.wait_response(&id)?;
            for thread in response
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(summary) = summary(locator_root, thread, archived)? {
                    sessions.push(summary);
                }
            }
        }
        Ok(sessions)
    })
}

pub(in crate::modules::agents::adapter) fn rename_session(
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    with_connection(|connection| {
        let id = connection.send_request(
            "thread/name/set",
            json!({"threadId": session_id, "name": name}),
        )?;
        connection.wait_response::<Value>(&id).map(|_| ())
    })
}

pub(in crate::modules::agents::adapter) fn delete_session(session_id: &str) -> Result<(), String> {
    with_connection(|connection| {
        let id = connection.send_request("thread/delete", json!({"threadId": session_id}))?;
        connection.wait_response::<Value>(&id).map(|_| ())
    })
}

pub(in crate::modules::agents::adapter) fn load_history(
    path: &Path,
) -> Result<DiscoveredHistory, String> {
    let locator = external_session_locator("codex-cli", path)
        .ok_or_else(|| format!("invalid Codex session locator: {}", path.display()))?;
    with_connection(|connection| {
        let id = connection.send_request(
            "thread/read",
            json!({"threadId": locator, "includeTurns": true}),
        )?;
        let response: Value = connection.wait_response(&id)?;
        let thread = response.get("thread").unwrap_or(&response);
        let mut messages = Vec::new();
        for turn in thread
            .get("turns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for item in turn
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(message) = history_message(item) {
                    messages.push(json!({"type": "message", "message": message}));
                }
            }
        }
        let model = string(thread, &["model"]).map(|model| ("openai".to_owned(), model.to_owned()));
        let thinking_level = string(thread, &["effort", "reasoningEffort"]).map(str::to_owned);
        Ok(DiscoveredHistory {
            messages,
            model,
            thinking_level,
        })
    })
}

fn with_connection<T>(
    operation: impl FnOnce(
        &mut CodexConnection<BufReader<ChildStdout>, ChildStdin>,
    ) -> Result<T, String>,
) -> Result<T, String> {
    let program = std::env::var_os("FARCASTER_CODEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "codex".into());
    let mut command = Command::new(program);
    command.args(["app-server", "--stdio"]);
    super::configure_permissions(&mut command);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start Codex catalog app-server: {error}"))?;
    child_stderr::capture(&mut child, "codex-catalog")?;
    let result = connect(&mut child).and_then(|mut connection| operation(&mut connection));
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn connect(
    child: &mut Child,
) -> Result<CodexConnection<BufReader<ChildStdout>, ChildStdin>, String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex catalog stdin must be piped".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex catalog stdout must be piped".to_owned())?;
    let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
    connection.initialize(CodexClientInfo {
        name: "farcaster-catalog".into(),
        title: Some("Farcaster".into()),
        version: env!("CARGO_PKG_VERSION").into(),
    })?;
    Ok(connection)
}

fn summary(
    locator_root: &Path,
    thread: &Value,
    archived: bool,
) -> Result<Option<DiscoveredSession>, String> {
    let Some(id) = string(thread, &["id"]) else {
        return Ok(None);
    };
    let Some(cwd) = string(thread, &["cwd"]) else {
        return Ok(None);
    };
    let project = PathBuf::from(cwd);
    if !project.is_dir() {
        return Ok(None);
    }
    let title = string(thread, &["name", "title", "preview"])
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("New Codex session")
        .to_owned();
    let first_user_message = string(thread, &["preview"]).unwrap_or_default().to_owned();
    let modified = timestamp(
        thread,
        &["updatedAt", "updated_at", "createdAt", "created_at"],
    );
    let timestamp = string(thread, &["createdAt", "created_at"])
        .unwrap_or_default()
        .to_owned();
    let parent_session = string(thread, &["parentThreadId", "parent_thread_id"]).map(str::to_owned);
    let is_running = status(thread).is_some_and(|status| {
        matches!(status, "active" | "running" | "inProgress" | "in_progress")
    });
    let path = external_session_path(locator_root, "codex-cli", id);
    let search = format!("{title} {first_user_message} {cwd} codex");
    Ok(Some(DiscoveredSession {
        id: id.to_owned(),
        harness: "codex-cli".into(),
        path,
        project,
        title,
        first_user_message,
        timestamp,
        parent_session,
        modified,
        message_count: thread
            .get("turns")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        usage: codex_usage(thread),
        archived,
        is_running,
        search,
    }))
}

fn codex_usage(thread: &Value) -> DiscoveredUsage {
    let usage = thread
        .pointer("/tokenUsage/total")
        .or_else(|| thread.pointer("/usage/total"));
    let Some(usage) = usage else {
        return DiscoveredUsage::default();
    };
    let reported_input = usage
        .get("inputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("outputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = usage
        .get("cachedInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = usage
        .get("cacheWriteInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input = reported_input.saturating_sub(cache_read.saturating_add(cache_write));
    DiscoveredUsage {
        input,
        output,
        cache_read,
        cache_write,
        total: input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write),
        cost_micros: 0,
    }
}

fn history_message(item: &Value) -> Option<Value> {
    match item.get("type").and_then(Value::as_str)? {
        "userMessage" => Some(json!({
            "role": "user",
            "content": text_content(item.get("content")),
        })),
        "agentMessage" => Some(json!({
            "role": "assistant",
            "content": [{"type": "text", "text": string(item, &["text"]).unwrap_or_default()}],
        })),
        "reasoning" => Some(json!({
            "role": "assistant",
            "content": [{"type": "thinking", "thinking": reasoning_text(item)}],
        })),
        _ => None,
    }
}

fn text_content(content: Option<&Value>) -> Vec<Value> {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| {
            let text = string(part, &["text"])?;
            Some(json!({"type": "text", "text": text}))
        })
        .collect()
}

fn reasoning_text(item: &Value) -> String {
    string(item, &["text"])
        .map(str::to_owned)
        .or_else(|| {
            item.get("summary").and_then(Value::as_array).map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| string(part, &["text"]))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .unwrap_or_default()
}

fn status(value: &Value) -> Option<&str> {
    value
        .get("status")
        .and_then(|status| status.as_str().or_else(|| status.get("type")?.as_str()))
}

fn string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key)?.as_str())
}

fn timestamp(value: &Value, keys: &[&str]) -> SystemTime {
    let raw = keys.iter().find_map(|key| value.get(*key));
    let seconds = raw
        .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()))
        .unwrap_or(0);
    if seconds == 0 {
        SystemTime::now()
    } else {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_thread_metadata() -> Result<(), String> {
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let value = json!({
            "id": "thread-1",
            "cwd": project,
            "name": "Fix tests",
            "preview": "Please fix tests",
            "updatedAt": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs(),
            "status": {"type": "active"},
            "tokenUsage": {"total": {"inputTokens": 100, "outputTokens": 20, "cachedInputTokens": 80}},
        });
        let session = summary(project.as_path(), &value, false)?.ok_or("summary")?;
        assert_eq!(session.harness, "codex-cli");
        assert!(session.is_running);
        assert_eq!(session.title, "Fix tests");
        assert_eq!(session.usage.input, 20);
        assert_eq!(session.usage.output, 20);
        assert_eq!(session.usage.cache_read, 80);
        assert_eq!(session.usage.total, 120);
        Ok(())
    }

    #[test]
    fn translates_thread_messages() {
        assert_eq!(
            history_message(&json!({
                "type": "userMessage",
                "content": [{"type": "text", "text": "hello"}],
            }))
            .expect("user message")["role"],
            "user"
        );
        assert_eq!(
            history_message(&json!({"type": "agentMessage", "text": "done"}))
                .expect("agent message")["role"],
            "assistant"
        );
    }
}
