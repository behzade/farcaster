use std::{
    collections::{HashMap, VecDeque},
    process::Stdio,
    sync::mpsc,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::server::OpenCodeServerProcess;
use crate::{
    access::SandboxedCommand,
    agents::{
        AgentLaunchConfig, WorkerContext, WorkerEvent, WorkerInput, WorkerInputResponse,
        WorkerLaunch,
        WorkerSendMode, WorkerSession, WorkerSessionFactory,
    },
    modules::agents::adapter::{child_stderr, farcaster_mcp},
};

#[derive(Clone)]
pub(crate) struct OpenCodeWorkerFactory {
    command: AgentLaunchConfig,
}

impl OpenCodeWorkerFactory {
    pub(crate) fn new(command: AgentLaunchConfig) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for OpenCodeWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("OpenCode worker provider and model must be supplied together".into());
        }
        let mut sandbox = self.command.command(&launch.project)?;
        let caller_identity =
            crate::modules::agents::core::CallerRegistry::shared().issue(&launch.project);
        configure_farcaster_mcp(&mut sandbox.command, caller_identity.token())?;
        let password = worker_password()?;
        let mut child = sandbox
            .command
            .args(["serve", "--stdio", "--print-logs"])
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start OpenCode worker server: {error}"))?;
        child_stderr::capture(&mut child, "opencode-worker")?;
        let server = OpenCodeServerProcess::attach(child, "opencode", password)?;
        let mut client = server.client();
        let selected_model = launch
            .provider
            .as_deref()
            .zip(launch.model.as_deref())
            .map(|(provider, model)| (provider, model, launch.effort.as_deref()));
        let session = match launch.context {
            WorkerContext::Fresh => client.create_session(
                &launch.project.to_string_lossy(),
                Some(&launch.parent_session),
                selected_model,
            )?,
            WorkerContext::Session { session_locator } => {
                if session_locator != launch.parent_session {
                    return Err(
                        "OpenCode workers cannot inherit context from a session other than their parent"
                            .into(),
                    );
                }
                client.fork_session(&session_locator, selected_model)?
            }
        };
        let session_id = session.id;
        let incoming = start_event_reader(&server, &session_id)?;
        caller_identity.bind(session_id.clone());
        Ok(Box::new(OpenCodeWorkerSession {
            _caller_identity: caller_identity,
            _sandbox: sandbox,
            server,
            session_id: session_id.clone(),
            provider: launch.provider,
            model: launch.model,
            effort: launch.effort,
            incoming,
            message_started: false,
            reasoning_started: false,
            pending_inputs: HashMap::new(),
            generation: 0,
            completions: None,
            pending: VecDeque::from([WorkerEvent::SessionChanged {
                locator: session_id,
            }]),
        }))
    }
}

pub(in crate::modules::agents::adapter) fn spawn_main(
    command: &AgentLaunchConfig,
    launch: &crate::agents::SessionLaunch,
) -> Result<
    (
        Box<dyn WorkerSession>,
        String,
        crate::modules::agents::adapter::main_session::MainSessionMetadata,
    ),
    String,
> {
    let mut sandbox = command.command(&launch.project)?;
    let caller_identity =
        crate::modules::agents::core::CallerRegistry::shared().issue(&launch.project);
    configure_farcaster_mcp(&mut sandbox.command, caller_identity.token())?;
    let password = worker_password()?;
    let mut child = sandbox
        .command
        .args(["serve", "--stdio", "--print-logs"])
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start OpenCode main-session server: {error}"))?;
    child_stderr::capture(&mut child, "opencode-main-session")?;
    let server = OpenCodeServerProcess::attach(child, "opencode", password)?;
    let mut client = server.client();
    let metadata = load_main_metadata(&mut client, &launch.project.to_string_lossy())?;
    let session = match &launch.start {
        crate::agents::SessionStart::New => client.create_session(
            &launch.project.to_string_lossy(),
            None,
            None,
        )?,
        crate::agents::SessionStart::Resume(_) => client.get_session(
            launch
                .session_id
                .as_deref()
                .ok_or_else(|| "OpenCode resume requires a session id".to_owned())?,
        )?,
        crate::agents::SessionStart::Fork(_) => client.fork_session(
            launch
                .session_id
                .as_deref()
                .ok_or_else(|| "OpenCode fork requires a session id".to_owned())?,
            None,
        )?,
    };
    let session_id = session.id;
    let incoming = start_event_reader(&server, &session_id)?;
    caller_identity.bind(session_id.clone());
    Ok((
        Box::new(OpenCodeWorkerSession {
            _caller_identity: caller_identity,
            _sandbox: sandbox,
            server,
            session_id: session_id.clone(),
            provider: None,
            model: None,
            effort: None,
            incoming,
            message_started: false,
            reasoning_started: false,
            pending_inputs: HashMap::new(),
            generation: 0,
            completions: None,
            pending: VecDeque::new(),
        }),
        session_id,
        metadata,
    ))
}

fn load_main_metadata(
    client: &mut super::client::OpenCodeClient<super::transport::OpenCodeTcpTransport>,
    directory: &str,
) -> Result<crate::modules::agents::adapter::main_session::MainSessionMetadata, String> {
    let model_response = client.models(directory)?;
    let model_rows = model_response
        .as_array()
        .or_else(|| model_response.get("data").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    let mut efforts = Vec::new();
    let models = model_rows
        .iter()
        .filter(|model| model.get("enabled").and_then(Value::as_bool) != Some(false))
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?;
            let provider = model
                .get("providerID")
                .and_then(Value::as_str)
                .unwrap_or("opencode");
            for effort in model
                .get("variants")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|variant| {
                    variant
                        .as_str()
                        .or_else(|| variant.get("id")?.as_str())
                })
            {
                if !efforts.iter().any(|known| known == effort) {
                    efforts.push(effort.to_owned());
                }
            }
            Some(json!({
                "id": id,
                "name": model.get("name").and_then(Value::as_str).unwrap_or(id),
                "provider": provider,
                "contextWindow": model.pointer("/limit/context").and_then(Value::as_u64).unwrap_or(0),
                "reasoning": true,
            }))
        })
        .collect();
    let agent_response = client.agents(directory)?;
    let agent_rows = agent_response
        .as_array()
        .or_else(|| agent_response.get("data").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    let modes = agent_rows
        .iter()
        .filter(|agent| agent.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter(|agent| {
            agent
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| matches!(mode, "primary" | "all"))
        })
        .filter_map(|agent| {
            let id = agent.get("id")?.as_str()?;
            Some(json!({
                "id": id,
                "name": agent.get("name").and_then(Value::as_str).unwrap_or(id),
                "description": agent.get("description").and_then(Value::as_str),
            }))
        })
        .collect();
    let command_response = client.commands(directory)?;
    let command_rows = command_response
        .as_array()
        .or_else(|| command_response.get("data").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    let commands = command_rows
        .iter()
        .filter_map(|command| {
            Some(json!({
                "name": command.get("name")?.as_str()?,
                "description": command.get("description").and_then(Value::as_str),
                "source": "prompt",
            }))
        })
        .collect();
    Ok(crate::modules::agents::adapter::main_session::MainSessionMetadata {
        models,
        efforts,
        commands,
        modes,
    })
}

enum PendingOpenCodeInput {
    Permission,
    Form {
        key: String,
        values: HashMap<String, String>,
    },
}

struct OpenCodeWorkerSession {
    _caller_identity: crate::modules::agents::core::CallerIdentity,
    _sandbox: SandboxedCommand,
    server: OpenCodeServerProcess,
    session_id: String,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    incoming: mpsc::Receiver<Result<super::contract::OpenCodeEvent, String>>,
    message_started: bool,
    reasoning_started: bool,
    pending_inputs: HashMap<String, PendingOpenCodeInput>,
    generation: u64,
    completions: Option<mpsc::Receiver<(u64, Result<String, String>)>>,
    pending: VecDeque<WorkerEvent>,
}

impl OpenCodeWorkerSession {
    fn send_prompt(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        files: Vec<super::contract::OpenCodeFileInput>,
    ) -> Result<(), String> {
        let delivery = match mode {
            WorkerSendMode::Prompt | WorkerSendMode::Queue => {
                super::contract::OpenCodeDelivery::Queue
            }
            WorkerSendMode::Steer => super::contract::OpenCodeDelivery::Steer,
        };
        self.server
            .client()
            .prompt(&self.session_id, &message, files, delivery)?;
        if mode != WorkerSendMode::Steer {
            self.message_started = false;
            self.reasoning_started = false;
        }
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let session_id = self.session_id.clone();
        let mut client = self.server.client();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name(format!("opencode-worker-{session_id}"))
            .spawn(move || {
                let result = client
                    .wait_session(&session_id)
                    .and_then(|()| client.context(&session_id))
                    .map(|context| final_assistant_text(&context));
                let _ = sender.send((generation, result));
            })
            .map_err(|error| format!("watch OpenCode worker: {error}"))?;
        self.completions = Some(receiver);
        self.pending.push_back(WorkerEvent::Started);
        Ok(())
    }

    fn poll_native_event(&mut self) -> Option<WorkerEvent> {
        loop {
            let event = match self.incoming.try_recv().ok()? {
                Ok(event) => event,
                Err(error) => return Some(WorkerEvent::Failed(error)),
            };
            let event_session = event
                .data
                .get("sessionID")
                .and_then(Value::as_str)
                .or_else(|| event.data.pointer("/form/sessionID").and_then(Value::as_str));
            if event_session != Some(self.session_id.as_str()) {
                continue;
            }
            match event.event.as_deref()? {
                "session.text.delta" => {
                    let delta = event.data.get("delta").and_then(Value::as_str)?;
                    let index = usize::from(self.reasoning_started);
                    let update = WorkerEvent::Activity(json!({
                        "type": "message_update",
                        "assistantMessageEvent": {
                            "type": "text_delta",
                            "contentIndex": index,
                            "delta": delta,
                        }
                    }));
                    if !self.message_started {
                        self.message_started = true;
                        self.pending.push_back(update);
                        return Some(WorkerEvent::Activity(json!({
                            "type": "message_start",
                            "message": {"role": "assistant", "content": []}
                        })));
                    }
                    return Some(update);
                }
                "session.reasoning.delta" => {
                    let delta = event.data.get("delta").and_then(Value::as_str)?;
                    if !self.message_started {
                        self.message_started = true;
                        self.reasoning_started = true;
                        self.pending.push_back(WorkerEvent::Activity(json!({
                            "type": "message_update",
                            "assistantMessageEvent": {
                                "type": "thinking_start",
                                "contentIndex": 0,
                            }
                        })));
                        self.pending.push_back(WorkerEvent::Activity(json!({
                            "type": "message_update",
                            "assistantMessageEvent": {
                                "type": "thinking_delta",
                                "contentIndex": 0,
                                "delta": delta,
                            }
                        })));
                        return Some(WorkerEvent::Activity(json!({
                            "type": "message_start",
                            "message": {"role": "assistant", "content": []}
                        })));
                    }
                    self.reasoning_started = true;
                    return Some(WorkerEvent::Activity(json!({
                        "type": "message_update",
                        "assistantMessageEvent": {
                            "type": "thinking_delta",
                            "contentIndex": 0,
                            "delta": delta,
                        }
                    })));
                }
                "session.tool.input.started" => {
                    return Some(WorkerEvent::Activity(json!({
                        "type": "tool_execution_start",
                        "toolCallId": event.data.get("id").and_then(Value::as_str)?,
                        "toolName": event.data.get("name").and_then(Value::as_str).unwrap_or("tool"),
                        "args": {},
                    })));
                }
                "session.tool.called" => {
                    return Some(WorkerEvent::Activity(json!({
                        "type": "tool_execution_update",
                        "toolCallId": event.data.get("id").and_then(Value::as_str)?,
                        "partialResult": {"content": [{
                            "type": "text",
                            "text": event.data.get("input").map(Value::to_string).unwrap_or_default(),
                        }]},
                    })));
                }
                "session.tool.progress" => {
                    return Some(WorkerEvent::Activity(json!({
                        "type": "tool_execution_update",
                        "toolCallId": event.data.get("id").and_then(Value::as_str)?,
                        "partialResult": {"content": [{
                            "type": "text",
                            "text": event.data.get("metadata").map(Value::to_string).unwrap_or_default(),
                        }]},
                    })));
                }
                "session.step.ended" => {
                    let usage = event
                        .data
                        .get("tokens")
                        .or_else(|| event.data.get("usage"))?;
                    let input = usage.get("input").and_then(Value::as_u64).unwrap_or(0);
                    let output = usage.get("output").and_then(Value::as_u64).unwrap_or(0);
                    let reasoning = usage.get("reasoning").and_then(Value::as_u64).unwrap_or(0);
                    let cache_read = usage
                        .pointer("/cache/read")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cache_write = usage
                        .pointer("/cache/write")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    return Some(WorkerEvent::Activity(json!({
                        "type": "turn_end",
                        "usage": {
                            "input": input,
                            "output": output + reasoning,
                            "cacheRead": cache_read,
                            "cacheWrite": cache_write,
                            "totalTokens": input + output + reasoning + cache_read + cache_write,
                        }
                    })));
                }
                "session.tool.success" | "session.tool.failed" => {
                    let failed = event.event.as_deref() == Some("session.tool.failed");
                    return Some(WorkerEvent::Activity(json!({
                        "type": "tool_execution_end",
                        "toolCallId": event.data.get("id").and_then(Value::as_str)?,
                        "result": {"content": event.data.get("content").cloned().unwrap_or_else(|| json!([]))},
                        "isError": failed,
                    })));
                }
                "permission.asked" => {
                    let id = event.data.get("id").and_then(Value::as_str)?.to_owned();
                    let action = event
                        .data
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("OpenCode permission");
                    let resources = event
                        .data
                        .get("resources")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.pending_inputs
                        .insert(id.clone(), PendingOpenCodeInput::Permission);
                    return Some(WorkerEvent::NeedsInput(WorkerInput {
                        id,
                        prompt: if resources.is_empty() {
                            action.to_owned()
                        } else {
                            format!("{action}\n{resources}")
                        },
                        options: vec![
                            "Allow once".into(),
                            "Always allow".into(),
                            "Decline".into(),
                        ],
                        secret: false,
                    }));
                }
                "form.created" => {
                    let form = event.data.get("form")?;
                    let id = form.get("id").and_then(Value::as_str)?.to_owned();
                    let title = form
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("OpenCode question")
                        .to_owned();
                    let field = form.get("fields").and_then(Value::as_array)?.first()?;
                    let key = field.get("key").and_then(Value::as_str)?.to_owned();
                    let mut values = HashMap::new();
                    let options = field
                        .get("options")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|option| {
                            let value = option.get("value")?.as_str()?.to_owned();
                            let label = option
                                .get("label")
                                .and_then(Value::as_str)
                                .unwrap_or(&value)
                                .to_owned();
                            values.insert(label.clone(), value);
                            Some(label)
                        })
                        .collect();
                    self.pending_inputs.insert(
                        id.clone(),
                        PendingOpenCodeInput::Form { key, values },
                    );
                    return Some(WorkerEvent::NeedsInput(WorkerInput {
                        id,
                        prompt: title,
                        options,
                        secret: false,
                    }));
                }
                _ => {}
            }
        }
    }
}

impl WorkerSession for OpenCodeWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_prompt(message, mode, Vec::new())
    }

    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        let files = images
            .into_iter()
            .enumerate()
            .map(|(index, image)| super::contract::OpenCodeFileInput {
                uri: format!("data:{};base64,{}", image.mime_type, image.data),
                name: Some(format!("image-{}", index + 1)),
                description: None,
            })
            .collect();
        self.send_prompt(message, mode, files)
    }

    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        let pending = self
            .pending_inputs
            .remove(&response.id)
            .ok_or_else(|| format!("unknown OpenCode interaction: {}", response.id))?;
        let mut client = self.server.client();
        match pending {
            PendingOpenCodeInput::Permission => {
                let value = response.value.as_deref().unwrap_or_default().to_ascii_lowercase();
                let reply = if response.cancel || value.contains("decline") {
                    "reject"
                } else if value.contains("always") {
                    "always"
                } else {
                    "once"
                };
                client.reply_permission(&self.session_id, &response.id, reply)
            }
            PendingOpenCodeInput::Form { key, values } => {
                if response.cancel {
                    return client.cancel_form(&self.session_id, &response.id);
                }
                let value = response.value.unwrap_or_default();
                let value = values.get(&value).cloned().unwrap_or(value);
                client.reply_form(
                    &self.session_id,
                    &response.id,
                    json!({key: value}),
                )
            }
        }
    }

    fn abort(&mut self) -> Result<(), String> {
        self.generation = self.generation.saturating_add(1);
        self.server.client().interrupt(&self.session_id)
    }

    fn compact(&mut self) -> Result<(), String> {
        self.server.client().compact_session(&self.session_id)
    }

    fn rename(&mut self, name: &str) -> Result<(), String> {
        self.server.client().rename_session(&self.session_id, name)
    }

    fn select_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        self.server.client().select_model(
            &self.session_id,
            provider,
            model,
            self.effort.as_deref(),
        )?;
        self.provider = Some(provider.to_owned());
        self.model = Some(model.to_owned());
        Ok(())
    }

    fn select_effort(&mut self, effort: &str) -> Result<(), String> {
        self.effort = Some(effort.to_owned());
        if let (Some(provider), Some(model)) = (self.provider.as_deref(), self.model.as_deref()) {
            self.server
                .client()
                .select_model(&self.session_id, provider, model, Some(effort))?;
        }
        Ok(())
    }

    fn select_mode(&mut self, mode: &str) -> Result<(), String> {
        self.server.client().select_agent(&self.session_id, mode)
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        if let Some(event) = self.poll_native_event() {
            return Some(event);
        }
        let completion = self.completions.as_ref()?.try_recv().ok()?;
        if completion.0 != self.generation {
            return None;
        }
        self.completions = None;
        Some(match completion.1 {
            Ok(output) => {
                if self.message_started {
                    self.pending.push_back(WorkerEvent::Settled { output });
                    WorkerEvent::Activity(json!({
                        "type": "message_end",
                        "message": {"role": "assistant"}
                    }))
                } else {
                    WorkerEvent::Settled { output }
                }
            }
            Err(error) => WorkerEvent::Failed(error),
        })
    }

    fn close(&mut self) -> Result<(), String> {
        self.server.terminate()
    }
}

fn start_event_reader(
    server: &OpenCodeServerProcess,
    session_id: &str,
) -> Result<mpsc::Receiver<Result<super::contract::OpenCodeEvent, String>>, String> {
    let mut stream = server.event_stream()?;
    let (sender, receiver) = mpsc::channel();
    let name = session_id.to_owned();
    thread::Builder::new()
        .name(format!("opencode-events-{name}"))
        .spawn(move || loop {
            match stream.next() {
                Ok(Some(event)) => {
                    if sender.send(Ok(event)).is_err() {
                        return;
                    }
                }
                Ok(None) => {
                    let _ = sender.send(Err("OpenCode event stream closed".into()));
                    return;
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            }
        })
        .map_err(|error| format!("start OpenCode event reader: {error}"))?;
    Ok(receiver)
}

fn configure_farcaster_mcp(
    command: &mut std::process::Command,
    caller_token: &str,
) -> Result<(), String> {
    let existing = command
        .get_envs()
        .find(|(name, _)| *name == "OPENCODE_CONFIG_CONTENT")
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned());
    let mut config = existing.map_or_else(
        || Ok(serde_json::json!({})),
        |value| {
            serde_json::from_str::<Value>(&value)
                .map_err(|error| format!("parse OPENCODE_CONFIG_CONTENT: {error}"))
        },
    )?;
    if !config.is_object() {
        return Err("OPENCODE_CONFIG_CONTENT must be a JSON object".into());
    }
    merge_json(
        &mut config,
        serde_json::json!({
            "mcp": {
                "servers": {
                    "farcaster": {
                        "type": "remote",
                        "url": farcaster_mcp::URL,
                        "headers": {(farcaster_mcp::CALLER_HEADER): caller_token},
                        "oauth": false,
                        "codemode": false
                    }
                }
            }
        }),
    );
    command.env(
        "OPENCODE_CONFIG_CONTENT",
        serde_json::to_string(&config)
            .map_err(|error| format!("encode OpenCode MCP configuration: {error}"))?,
    );
    Ok(())
}

fn merge_json(target: &mut Value, overlay: Value) {
    match (target, overlay) {
        (Value::Object(target), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge_json(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, overlay) => *target = overlay,
    }
}

fn final_assistant_text(context: &[Value]) -> String {
    context
        .iter()
        .rev()
        .find(|message| message["type"].as_str() == Some("assistant"))
        .and_then(|message| message["content"].as_array())
        .map(|content| {
            content
                .iter()
                .filter_map(|part| {
                    (part["type"].as_str() == Some("text"))
                        .then(|| part["text"].as_str())
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn worker_password() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is unavailable".to_owned())?
        .as_nanos();
    Ok(format!("farcaster-{}-{nanos}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_startup_merges_direct_farcaster_mcp() {
        let mut command = std::process::Command::new("opencode2");
        command.env(
            "OPENCODE_CONFIG_CONTENT",
            r#"{"model":"provider/model","mcp":{"servers":{"other":{"type":"remote","url":"https://example.test/mcp"}}}}"#,
        );
        configure_farcaster_mcp(&mut command, "caller-1").expect("MCP config");
        let value = command
            .get_envs()
            .find(|(name, _)| *name == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, value)| value)
            .and_then(|value| serde_json::from_str::<Value>(&value.to_string_lossy()).ok())
            .expect("inline config");
        assert_eq!(value["model"], "provider/model");
        assert_eq!(
            value["mcp"]["servers"]["other"]["url"],
            "https://example.test/mcp"
        );
        assert_eq!(
            value["mcp"]["servers"]["farcaster"]["url"],
            farcaster_mcp::URL
        );
        assert_eq!(
            value["mcp"]["servers"]["farcaster"]["headers"][farcaster_mcp::CALLER_HEADER],
            "caller-1"
        );
        assert_eq!(value["mcp"]["servers"]["farcaster"]["codemode"], false);
        assert_eq!(value["mcp"]["servers"]["farcaster"]["oauth"], false);
    }

    #[test]
    fn extracts_the_last_assistant_text() {
        let context = [
            json!({"type":"assistant","content":[{"type":"text","text":"old"}]}),
            json!({"type":"user","text":"next"}),
            json!({"type":"assistant","content":[{"type":"reasoning","text":"hidden"},{"type":"text","text":"done"}]}),
        ];
        assert_eq!(final_assistant_text(&context), "done");
    }
}
