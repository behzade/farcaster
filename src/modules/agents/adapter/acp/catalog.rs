use std::{
    io::BufReader,
    path::Path,
    process::{ChildStdin, ChildStdout, Stdio},
};

use serde_json::{Value, json};

use super::{
    AcpProfile,
    connection::AcpConnection,
    translate::{
        commands_from_update, merge_tool_metadata, merged_tool_content, metadata_from_session,
        normalize_tool_name, tool_args, tool_metadata,
    },
    wire::AcpInbound,
    worker::configure_command,
};
use crate::agents::{AgentLaunchConfig, DiscoveredHistory, HarnessAccessMode, ToolMetadata};

use super::super::{child_stderr, main_session};

pub(in crate::modules::agents::adapter) fn load_configuration(
    profile: &AcpProfile,
    project: &Path,
) -> Result<(main_session::MainSessionMetadata, String), String> {
    with_connection(profile, project, |connection| {
        let id = connection.send_request(
            "session/new",
            json!({"cwd": project.to_string_lossy(), "mcpServers": []}),
        )?;
        let response = connection.wait_response(&id)?;
        let session_id = response
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} did not provide an ACP session id", profile.name))?;
        let (mut metadata, _) = metadata_from_session(profile, &response);
        if let Some(commands) = connection
            .drain_queued()
            .iter()
            .filter_map(|message| commands_from_update(message, session_id))
            .next_back()
        {
            metadata.commands = commands;
        }
        Ok((metadata, session_id.to_owned()))
    })
}

pub(in crate::modules::agents::adapter) fn load_history(
    profile: &AcpProfile,
    path: &Path,
    project: &Path,
) -> Result<DiscoveredHistory, String> {
    let locator =
        main_session::external_session_locator(profile.backend, path).ok_or_else(|| {
            format!(
                "invalid {} session locator: {}",
                profile.name,
                path.display()
            )
        })?;
    with_connection(profile, project, |connection| {
        let id = connection.send_request(
            "session/load",
            json!({
                "sessionId": locator,
                "cwd": project.to_string_lossy(),
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

type CatalogConnection = AcpConnection<BufReader<ChildStdout>, ChildStdin>;

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

#[derive(Default)]
struct HistoryToolState {
    index: usize,
    name: String,
    metadata: ToolMetadata,
    finished: bool,
}

fn replay_history(messages: impl IntoIterator<Item = AcpInbound>) -> Vec<Value> {
    let mut history = Vec::new();
    let mut last_chunk = None;
    let mut tools = std::collections::HashMap::<String, HistoryToolState>::new();
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
            Some("tool_call" | "tool_call_update") => {
                last_chunk = None;
                let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
                    continue;
                };
                if !tools.contains_key(id) {
                    let metadata = tool_metadata(update);
                    let title = metadata.title.as_deref().unwrap_or("tool");
                    let native = metadata.native.as_ref().unwrap_or(update);
                    let name = normalize_tool_name(native, title);
                    let index = history.len();
                    history.push(json!({
                        "role":"assistant",
                        "content":[{
                            "type":"toolCall",
                            "id":id,
                            "name":name,
                            "arguments":tool_args(&metadata),
                            "toolMetadata":metadata,
                        }],
                    }));
                    tools.insert(
                        id.to_owned(),
                        HistoryToolState {
                            index,
                            name,
                            metadata,
                            finished: false,
                        },
                    );
                } else {
                    let state = tools.get_mut(id).expect("tool checked above");
                    merge_tool_metadata(&mut state.metadata, update);
                    if let Some(call) = history
                        .get_mut(state.index)
                        .and_then(|message| message.pointer_mut("/content/0"))
                    {
                        call["arguments"] = tool_args(&state.metadata);
                        call["toolMetadata"] = json!(state.metadata);
                    }
                }

                let state = tools.get_mut(id).expect("tool inserted above");
                let status = update.get("status").and_then(Value::as_str);
                if matches!(status, Some("completed" | "failed")) && !state.finished {
                    state.finished = true;
                    history.push(json!({
                        "role":"toolResult",
                        "toolCallId":id,
                        "toolName":state.name,
                        "content":merged_tool_content(&state.metadata, update),
                        "isError":status == Some("failed"),
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
            notification(json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"Read"})),
            notification(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","kind":"read","locations":[{"path":"README.md"}],"rawInput":{"path":"README.md"}}),
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
        assert_eq!(history[1].pointer("/content/0/name"), Some(&json!("Read")));
        assert_eq!(
            history[1].pointer("/content/0/arguments/path"),
            Some(&json!("README.md"))
        );
        assert_eq!(
            history[1].pointer("/content/0/toolMetadata/category"),
            Some(&json!("read"))
        );
        assert_eq!(
            history[1].pointer("/content/0/toolMetadata/targets/0"),
            Some(&json!("README.md"))
        );
        assert_eq!(
            history[1].pointer("/content/0/toolMetadata/native/title"),
            Some(&json!("Read"))
        );
        assert_eq!(history[2]["role"], "toolResult");
    }

    #[test]
    fn replay_retains_output_emitted_before_completion() {
        let notification = |update| AcpInbound::Notification {
            method: "session/update".into(),
            params: json!({"update":update}),
        };
        let history = replay_history([
            notification(json!({
                "sessionUpdate":"tool_call",
                "toolCallId":"partial",
                "title":"Long task"
            })),
            notification(json!({
                "sessionUpdate":"tool_call_update",
                "toolCallId":"partial",
                "content":[{"type":"text","text":"output before completion"}]
            })),
            notification(json!({
                "sessionUpdate":"tool_call_update",
                "toolCallId":"partial",
                "status":"completed"
            })),
        ]);
        assert_eq!(
            history[1].pointer("/content/0/text"),
            Some(&json!("output before completion"))
        );
    }

    #[test]
    fn replay_handles_completed_tool_as_its_first_update() {
        let history = replay_history([AcpInbound::Notification {
            method: "session/update".into(),
            params: json!({"update":{
                "sessionUpdate":"tool_call_update",
                "toolCallId":"done",
                "kind":"fetch",
                "title":"Fetch docs",
                "status":"completed",
                "rawInput":{"url":"https://example.com"},
                "content":[{"type":"text","text":"done"}]
            }}),
        }]);
        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0].pointer("/content/0/name"),
            Some(&json!("web_fetch"))
        );
        assert_eq!(
            history[0].pointer("/content/0/toolMetadata/category"),
            Some(&json!("fetch"))
        );
        assert_eq!(history[1]["role"], "toolResult");
    }
}
