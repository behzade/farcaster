use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{Mutex, OnceLock, mpsc},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use super::{
    AcpProfile,
    connection::AcpConnection,
    events::AcpInbound,
    translate::{
        commands_from_update, merge_tool_metadata, metadata_from_session, normalize_tool_name,
        tool_args, tool_metadata, tool_result,
    },
    worker::configure_command,
};
use crate::agents::{AgentLaunchConfig, DiscoveredHistory, HarnessAccessMode, ToolMetadata};

use super::super::{child_stderr, main_session};

pub(in crate::modules::agents::adapter) fn load_configuration(
    profile: &AcpProfile,
    project: &Path,
) -> Result<(main_session::MainSessionMetadata, String), String> {
    with_connection(profile, project, |connection, profile, project| {
        let response = connection.request_blocking(
            "session/new",
            json!({"cwd": project.to_string_lossy(), "mcpServers": []}),
        )?;
        let session_id = response
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} did not provide an ACP session id", profile.name))?;
        let (mut metadata, _) = metadata_from_session(profile, &response);
        if let Some(commands) = connection
            .drain_queued()?
            .iter()
            .filter_map(|message| commands_from_update(message, session_id))
            .next_back()
        {
            metadata.commands = commands;
        }
        close_session(connection, session_id);
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
    with_connection(profile, project, move |connection, profile, project| {
        let response = connection.request_blocking(
            "session/load",
            json!({
                "sessionId": locator,
                "cwd": project.to_string_lossy(),
                "mcpServers": [],
            }),
        )?;
        let queued = connection.drain_queued()?;
        let history = discovered_history(profile, queued, &response, &locator);
        close_session(connection, &locator);
        Ok(history)
    })
}

struct CatalogProcess {
    child: Option<Child>,
    connection: Option<AcpConnection>,
    project: PathBuf,
}

impl CatalogProcess {
    fn take_parts(mut self) -> Option<(Child, AcpConnection)> {
        match (self.child.take(), self.connection.take()) {
            (Some(child), Some(connection)) => Some((child, connection)),
            (Some(mut child), None) => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
            _ => None,
        }
    }
}

impl Drop for CatalogProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn catalog_is_reusable(existing_project: &Path, project: &Path, running: bool) -> bool {
    running && existing_project == project
}

fn catalog_processes() -> &'static Mutex<HashMap<&'static str, CatalogProcess>> {
    static PROCESSES: OnceLock<Mutex<HashMap<&'static str, CatalogProcess>>> = OnceLock::new();
    PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_connection<T: Send + 'static>(
    profile: &AcpProfile,
    project: &Path,
    operation: impl FnOnce(&mut AcpConnection, &AcpProfile, &Path) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let mut processes = catalog_processes()
        .lock()
        .map_err(|error| format!("{} ACP catalog lock: {error}", profile.name))?;
    let reused = processes.remove(profile.backend).and_then(|mut process| {
        let running = matches!(process.child.as_mut().map(Child::try_wait), Some(Ok(None)));
        if catalog_is_reusable(&process.project, project, running) {
            process.take_parts()
        } else {
            None
        }
    });
    let initialized = reused.is_some();
    let (mut child, connection) = match reused {
        Some(parts) => parts,
        None => spawn_catalog_child(profile, project)?,
    };
    let profile_owned = profile.clone();
    let project_owned = project.to_owned();
    let result = run_catalog_operation(Duration::from_secs(30), move || {
        let mut connection = connection;
        let result = (|| {
            if !initialized {
                connection.initialize(&profile_owned)?;
            } else {
                connection.drain_queued()?;
            }
            operation(&mut connection, &profile_owned, &project_owned)
        })();
        (result, connection)
    });
    match result {
        Ok((Ok(value), connection)) => {
            processes.insert(
                profile.backend,
                CatalogProcess {
                    child: Some(child),
                    connection: Some(connection),
                    project: project.to_owned(),
                },
            );
            Ok(value)
        }
        Ok((Err(error), _)) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("{} ACP catalog: {error}", profile.name))
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("{} ACP catalog: {error}", profile.name))
        }
    }
}

fn spawn_catalog_child(
    profile: &AcpProfile,
    project: &Path,
) -> Result<(Child, AcpConnection), String> {
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
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} ACP catalog stdin must be piped", profile.name));
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} ACP catalog stdout must be piped", profile.name));
        }
    };
    match AcpConnection::new(
        blocking::Unblock::new(stdout),
        blocking::Unblock::new(stdin),
        None,
    ) {
        Ok(connection) => Ok((child, connection)),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

fn close_session(connection: &mut AcpConnection, session_id: &str) {
    let _ = connection.request_blocking("session/close", json!({"sessionId": session_id}));
}

fn run_catalog_operation<T: Send + 'static>(
    timeout: Duration,
    operation: impl FnOnce() -> (Result<T, String>, AcpConnection) + Send + 'static,
) -> Result<(Result<T, String>, AcpConnection), String> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("acp-catalog-handshake".into())
        .spawn(move || {
            let _ = sender.send(operation());
        })
        .map_err(|error| format!("start ACP catalog handshake: {error}"))?;
    match receiver.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "timed out loading configuration after {} seconds; check the agent's authentication and connection",
            timeout.as_secs()
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("ACP catalog handshake stopped unexpectedly".into())
        }
    }
}

#[cfg(test)]
fn run_with_timeout<T: Send + 'static>(
    timeout: Duration,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("acp-catalog-handshake".into())
        .spawn(move || {
            let _ = sender.send(operation());
        })
        .map_err(|error| format!("start ACP catalog handshake: {error}"))?;
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "timed out loading configuration after {} seconds; check the agent's authentication and connection",
            timeout.as_secs()
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("ACP catalog handshake stopped unexpectedly".into())
        }
    }
}

#[derive(Default)]
struct HistoryToolState {
    index: usize,
    name: String,
    metadata: ToolMetadata,
    finished: bool,
}

pub(super) fn discovered_history(
    profile: &AcpProfile,
    queued: impl IntoIterator<Item = AcpInbound>,
    response: &Value,
    session_id: &str,
) -> DiscoveredHistory {
    DiscoveredHistory {
        messages: replay_history_for_session(queued, Some(session_id)),
        model: selected_model(profile, response),
        thinking_level: selected_option(response, &["thought_level", "reasoning", "effort"]),
    }
}

#[cfg(test)]
fn replay_history(messages: impl IntoIterator<Item = AcpInbound>) -> Vec<Value> {
    replay_history_for_session(messages, None)
}

fn replay_history_for_session(
    messages: impl IntoIterator<Item = AcpInbound>,
    session_id: Option<&str>,
) -> Vec<Value> {
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
        if let Some(expected) = session_id
            && let Some(actual) = params.get("sessionId").and_then(Value::as_str)
            && actual != expected
        {
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
                    let mut message = tool_result(&state.metadata, update);
                    message["role"] = json!("toolResult");
                    message["toolCallId"] = json!(id);
                    message["toolName"] = json!(state.name);
                    message["isError"] = json!(status == Some("failed"));
                    history.push(message);
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
#[path = "catalog_tests.rs"]
mod tests;
