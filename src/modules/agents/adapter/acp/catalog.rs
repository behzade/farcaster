use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::{ChildStdin, ChildStdout, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::{
    AcpProfile, connection::AcpConnection, translate::tool_content, wire::AcpInbound,
    worker::configure_command,
};
use crate::agents::{
    AgentLaunchConfig, DiscoveredHistory, DiscoveredSession, DiscoveredUsage, HarnessAccessMode,
};

use super::super::{child_stderr, main_session};

pub(in crate::modules::agents::adapter) fn discover(
    profile: &AcpProfile,
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let cwd =
        std::env::current_dir().map_err(|error| format!("resolve ACP catalog cwd: {error}"))?;
    with_connection(profile, &cwd, |connection| {
        Ok(list_sessions(connection)?
            .iter()
            .filter_map(|value| summary(profile, locator_root, value, query))
            .collect())
    })
}

pub(in crate::modules::agents::adapter) fn load_history(
    profile: &AcpProfile,
    path: &Path,
    project: Option<&Path>,
) -> Result<DiscoveredHistory, String> {
    let locator =
        main_session::external_session_locator(profile.backend, path).ok_or_else(|| {
            format!(
                "invalid {} session locator: {}",
                profile.name,
                path.display()
            )
        })?;
    let cwd = match project {
        Some(project) => project.to_owned(),
        None => session_project(profile, &locator)?
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
    };
    with_connection(profile, &cwd, |connection| {
        let id = connection.send_request(
            "session/load",
            json!({
                "sessionId": locator,
                "cwd": cwd.to_string_lossy(),
                "mcpServers": [],
            }),
        )?;
        let response = connection.wait_response(&id)?;
        let queued = connection.drain_queued();
        Ok(DiscoveredHistory {
            messages: replay_history(queued),
            model: selected_model(profile, &response),
            thinking_level: selected_option(&response, &["thought_level", "reasoning", "effort"]),
        })
    })
}

fn session_project(profile: &AcpProfile, locator: &str) -> Result<Option<PathBuf>, String> {
    let cwd =
        std::env::current_dir().map_err(|error| format!("resolve ACP catalog cwd: {error}"))?;
    with_connection(profile, &cwd, |connection| {
        Ok(list_sessions(connection)?
            .iter()
            .find(|session| session.get("sessionId").and_then(Value::as_str) == Some(locator))
            .and_then(|session| session.get("cwd").and_then(Value::as_str))
            .map(PathBuf::from))
    })
}

type CatalogConnection = AcpConnection<BufReader<ChildStdout>, ChildStdin>;

fn list_sessions(connection: &mut CatalogConnection) -> Result<Vec<Value>, String> {
    let mut cursor = None;
    let mut sessions = Vec::new();
    loop {
        let id = connection.send_request("session/list", json!({"cursor": cursor}))?;
        let response = connection.wait_response(&id)?;
        sessions.extend(
            response
                .get("sessions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .cloned(),
        );
        cursor = response
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if cursor.is_none() {
            return Ok(sessions);
        }
    }
}

fn with_connection<T>(
    profile: &AcpProfile,
    project: &Path,
    operation: impl FnOnce(&mut CatalogConnection) -> Result<T, String>,
) -> Result<T, String> {
    let config = AgentLaunchConfig {
        program: profile.program(),
        access_mode: HarnessAccessMode::Sandboxed,
        ..AgentLaunchConfig::default()
    };
    let mut command = config.command(project)?;
    configure_command(&mut command, profile, config.access_mode);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start {} ACP catalog: {error}", profile.name))?;
    child_stderr::capture(&mut child, "acp-catalog")?;
    let result = (|| {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("{} ACP catalog stdin must be piped", profile.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("{} ACP catalog stdout must be piped", profile.name))?;
        let mut connection = AcpConnection::new(BufReader::new(stdout), stdin);
        let initialized = connection.initialize(profile)?;
        if initialized.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
            return Err(format!("{} did not negotiate ACP version 1", profile.name));
        }
        operation(&mut connection)
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn summary(
    profile: &AcpProfile,
    locator_root: &Path,
    value: &Value,
    query: &str,
) -> Option<DiscoveredSession> {
    let id = value.get("sessionId")?.as_str()?;
    let cwd = value.get("cwd")?.as_str()?;
    let project = PathBuf::from(cwd);
    if !project.is_dir() || crate::projects::is_temporary_project(&project) {
        return None;
    }
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("New ACP session")
        .to_owned();
    let search = format!("{title} {cwd} {}", profile.name);
    if !query.is_empty()
        && !search
            .to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
    {
        return None;
    }
    let updated = value.get("updatedAt").or_else(|| value.get("createdAt"));
    Some(DiscoveredSession {
        id: id.into(),
        harness: profile.backend.into(),
        path: main_session::external_session_path(locator_root, profile.backend, id),
        project,
        title,
        first_user_message: String::new(),
        timestamp: value
            .get("createdAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        parent_session: crate::modules::agents::core::CallerRegistry::shared()
            .session_parent(profile.backend, id),
        modified: system_time(updated),
        message_count: 0,
        usage: DiscoveredUsage::default(),
        archived: false,
        is_running: false,
        search,
    })
}

fn system_time(value: Option<&Value>) -> SystemTime {
    let seconds = value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64()?.try_into().ok())
                .or_else(|| {
                    let parsed = time::OffsetDateTime::parse(
                        value.as_str()?,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .ok()?;
                    parsed.unix_timestamp().try_into().ok()
                })
        })
        .unwrap_or(0);
    if seconds == 0 {
        SystemTime::now()
    } else {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }
}

fn replay_history(messages: impl IntoIterator<Item = AcpInbound>) -> Vec<Value> {
    let mut history = Vec::new();
    let mut last_chunk = None;
    let mut tool_names = std::collections::HashMap::new();
    for message in messages {
        let AcpInbound::Notification { method, params } = message else {
            continue;
        };
        if method != "session/update" {
            continue;
        }
        let Some(update) = params.get("update") else {
            continue;
        };
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("user_message_chunk") => {
                if let Some(content) = update.get("content") {
                    append_history_chunk(
                        &mut history,
                        &mut last_chunk,
                        "user",
                        update.get("messageId").and_then(Value::as_str),
                        content.clone(),
                    );
                }
            }
            Some("agent_message_chunk") => {
                if let Some(content) = update.get("content") {
                    append_history_chunk(
                        &mut history,
                        &mut last_chunk,
                        "assistant",
                        update.get("messageId").and_then(Value::as_str),
                        content.clone(),
                    );
                }
            }
            Some("agent_thought_chunk") => {
                if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                    append_history_chunk(
                        &mut history,
                        &mut last_chunk,
                        "assistant",
                        update.get("messageId").and_then(Value::as_str),
                        json!({"type":"thinking", "thinking":text}),
                    );
                }
            }
            Some("tool_call") => {
                last_chunk = None;
                if let Some(id) = update.get("toolCallId").and_then(Value::as_str) {
                    let name = update
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("tool");
                    tool_names.insert(id.to_owned(), name.to_owned());
                    history.push(json!({
                        "role":"assistant",
                        "content":[{
                            "type":"toolCall",
                            "id":id,
                            "name":name,
                            "arguments":update.get("rawInput").cloned().unwrap_or_else(|| json!({})),
                        }],
                    }));
                }
            }
            Some("tool_call_update")
                if matches!(
                    update.get("status").and_then(Value::as_str),
                    Some("completed" | "failed")
                ) =>
            {
                last_chunk = None;
                if let Some(id) = update.get("toolCallId").and_then(Value::as_str) {
                    let failed = update.get("status").and_then(Value::as_str) == Some("failed");
                    history.push(json!({
                        "role":"toolResult",
                        "toolCallId":id,
                        "toolName":tool_names.get(id).map(String::as_str).unwrap_or("tool"),
                        "content":tool_content(update),
                        "isError":failed,
                    }));
                }
            }
            _ => {}
        }
    }
    history
}

fn append_history_chunk(
    history: &mut Vec<Value>,
    last_chunk: &mut Option<(String, Option<String>)>,
    role: &str,
    message_id: Option<&str>,
    content: Value,
) {
    let key = (role.to_owned(), message_id.map(str::to_owned));
    let can_merge = last_chunk.as_ref().is_some_and(|previous| {
        previous == &key || (previous.0 == role && previous.1.is_none() && key.1.is_none())
    });
    if can_merge
        && let Some(parts) = history
            .last_mut()
            .and_then(|message| message.get_mut("content"))
            .and_then(Value::as_array_mut)
    {
        if !append_content_text(parts, &content) {
            parts.push(content);
        }
        return;
    }
    history.push(json!({"role": role, "content": [content]}));
    *last_chunk = Some(key);
}

fn append_content_text(parts: &mut [Value], content: &Value) -> bool {
    let Some(last) = parts.last_mut() else {
        return false;
    };
    let field = match content.get("type").and_then(Value::as_str) {
        Some("text") => "text",
        Some("thinking") => "thinking",
        _ => return false,
    };
    if last.get("type") != content.get("type") {
        return false;
    }
    let Some(delta) = content.get(field).and_then(Value::as_str) else {
        return false;
    };
    let Some(Value::String(text)) = last.get_mut(field) else {
        return false;
    };
    text.push_str(delta);
    true
}

fn selected_model(profile: &AcpProfile, response: &Value) -> Option<(String, String)> {
    selected_option(response, &["model"]).map(|model| (profile.backend.into(), model))
}

fn selected_option(response: &Value, categories: &[&str]) -> Option<String> {
    response
        .get("configOptions")?
        .as_array()?
        .iter()
        .find(|option| {
            let category = option.get("category").and_then(Value::as_str).unwrap_or("");
            let id = option.get("id").and_then(Value::as_str).unwrap_or("");
            categories
                .iter()
                .any(|wanted| category == *wanted || id.contains(wanted))
        })?
        .get("currentValue")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::agents::adapter::acp::wire::AcpInbound;

    #[test]
    fn translates_acp_session_summaries() -> Result<(), String> {
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let profile = AcpProfile {
            backend: "example-acp",
            name: "Example",
            command: "example",
            path_environment: "EXAMPLE_PATH",
            arguments: &["acp"],
            auth_method: None,
            force_argument: None,
        };
        let session = summary(
            &profile,
            &project,
            &json!({"sessionId":"one", "cwd":project, "title":"Fix it", "updatedAt":1}),
            "",
        )
        .ok_or("summary")?;
        assert_eq!(session.harness, "example-acp");
        assert_eq!(session.title, "Fix it");
        Ok(())
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
            notification(
                json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"Read","rawInput":{"path":"README.md"}}),
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
        assert_eq!(history[2]["role"], "toolResult");
    }
}
