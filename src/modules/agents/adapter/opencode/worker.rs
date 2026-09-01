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
    agents::{
        AgentLaunchConfig, CommonTool, TokenUsage, WorkerActivity, WorkerActivityState,
        WorkerContext, WorkerEvent, WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode,
        WorkerSession, WorkerSessionFactory, WorkerUsage,
    },
    modules::agents::adapter::{child_stderr, farcaster_mcp, main_session},
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
        let mut prepared = self.command.command(&launch.project)?;
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue_as(
            &launch.project,
            crate::modules::agents::core::CallerProfile {
                backend: "opencode2".into(),
                provider: launch.provider.clone(),
                model: launch.model.clone(),
                effort: launch.effort.clone(),
            },
            None,
            launch.worker_id.clone(),
        );
        if farcaster_mcp::enabled() {
            configure_farcaster_mcp(&mut prepared, caller_identity.token())?;
        }
        let password = worker_password()?;
        configure_opencode_server(&mut prepared, self.command.access_mode)?;
        let mut child = prepared
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
            WorkerContext::Fresh => {
                client.create_session(&launch.project.to_string_lossy(), None, selected_model)?
            }
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
        let incoming = start_event_reader(&server, &session_id, None)?;
        caller_identity.bind(session_id.clone());
        Ok(Box::new(OpenCodeWorkerSession {
            caller_identity,
            server,
            session_id: session_id.clone(),
            provider: launch.provider,
            model: launch.model,
            effort: launch.effort,
            access_mode: self.command.access_mode,
            incoming,
            reasoning_started: false,
            session_usage: TokenUsage::default(),
            context_window: 0,
            pending_inputs: HashMap::new(),
            active_tools: HashMap::new(),
            generation: 0,
            completions: None,
            wake: None,
            pending: VecDeque::from([WorkerEvent::SessionChanged {
                locator: session_id,
            }]),
        }))
    }
}

pub(in crate::modules::agents::adapter) fn load_configuration(
    command: &AgentLaunchConfig,
    project: &std::path::Path,
) -> Result<crate::modules::agents::adapter::main_session::MainSessionMetadata, String> {
    let mut prepared = command.command(project)?;
    let password = worker_password()?;
    configure_opencode_server(&mut prepared, command.access_mode)?;
    let mut child = prepared
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start OpenCode catalog server: {error}"))?;
    child_stderr::capture(&mut child, "opencode-catalog")?;
    let mut server = OpenCodeServerProcess::attach(child, "opencode", password)?;
    let result = load_main_metadata(&mut server.client(), &project.to_string_lossy());
    let _ = server.terminate();
    result
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
    let mut prepared = command.command(&launch.project)?;
    let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue(
        &launch.project,
        crate::modules::agents::core::CallerProfile {
            backend: "opencode2".into(),
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
    );
    if farcaster_mcp::enabled() {
        configure_farcaster_mcp(&mut prepared, caller_identity.token())?;
    }
    let password = worker_password()?;
    configure_opencode_server(&mut prepared, command.access_mode)?;
    let mut child = prepared
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
    let context_window = metadata
        .models
        .first()
        .and_then(|model| model.get("contextWindow"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let session = match &launch.start {
        crate::agents::SessionStart::New => {
            client.create_session(&launch.project.to_string_lossy(), None, None)?
        }
        crate::agents::SessionStart::Resume(_) => {
            let session_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "OpenCode resume requires a session id".to_owned())?;
            client.get_session(&session_id)?
        }
        crate::agents::SessionStart::Fork(_) => {
            let session_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "OpenCode fork requires a session id".to_owned())?;
            client.fork_session(&session_id, None)?
        }
    };
    let session_id = session.id;
    let incoming = start_event_reader(&server, &session_id, launch.wake.clone())?;
    caller_identity.bind(session_id.clone());
    Ok((
        Box::new(OpenCodeWorkerSession {
            caller_identity,
            server,
            session_id: session_id.clone(),
            provider: None,
            model: None,
            effort: None,
            access_mode: command.access_mode,
            incoming,
            reasoning_started: false,
            session_usage: TokenUsage::default(),
            context_window,
            pending_inputs: HashMap::new(),
            active_tools: HashMap::new(),
            generation: 0,
            completions: None,
            wake: launch.wake.clone(),
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
    Ok(
        crate::modules::agents::adapter::main_session::MainSessionMetadata {
            models,
            efforts,
            commands,
            modes,
        },
    )
}

enum PendingOpenCodeInput {
    Permission {
        session_id: String,
    },
    Form {
        key: String,
        values: HashMap<String, String>,
    },
}

struct OpenCodeWorkerSession {
    caller_identity: crate::modules::agents::core::CallerIdentity,
    server: OpenCodeServerProcess,
    session_id: String,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    access_mode: crate::agents::HarnessAccessMode,
    incoming: mpsc::Receiver<Result<super::contract::OpenCodeEvent, String>>,
    reasoning_started: bool,
    session_usage: TokenUsage,
    context_window: u64,
    pending_inputs: HashMap<String, PendingOpenCodeInput>,
    active_tools: HashMap<String, String>,
    generation: u64,
    completions: Option<mpsc::Receiver<(u64, Result<String, String>)>>,
    wake: Option<thread::Thread>,
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
        self.caller_identity
            .set_activity(WorkerActivityState::Working);
        if mode != WorkerSendMode::Steer {
            self.reasoning_started = false;
        }
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        let session_id = self.session_id.clone();
        let mut client = self.server.client();
        let (sender, receiver) = mpsc::channel();
        let wake = self.wake.clone();
        thread::Builder::new()
            .name(format!("opencode-worker-{session_id}"))
            .spawn(move || {
                let result = client
                    .wait_session(&session_id)
                    .and_then(|()| client.context(&session_id))
                    .map(|context| final_assistant_text(&context));
                let _ = send_and_wake(&sender, (generation, result), wake.as_ref());
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
            if let Some((session_id, request_id)) = opencode_permission_request(&event) {
                if matches!(self.access_mode, crate::agents::HarnessAccessMode::Full) {
                    if let Err(error) = self
                        .server
                        .client()
                        .reply_permission(session_id, request_id, "once")
                    {
                        return Some(WorkerEvent::Failed(format!(
                            "approve OpenCode permission in full-access mode: {error}"
                        )));
                    }
                    continue;
                }
                self.pending_inputs.insert(
                    request_id.to_owned(),
                    PendingOpenCodeInput::Permission {
                        session_id: session_id.to_owned(),
                    },
                );
                return Some(WorkerEvent::NeedsInput(WorkerInput {
                    id: request_id.to_owned(),
                    prompt: opencode_permission_prompt(&event.data),
                    options: vec!["Allow once".into(), "Always allow".into(), "Decline".into()],
                    secret: false,
                }));
            }
            let event_session = event
                .data
                .get("sessionID")
                .and_then(Value::as_str)
                .or_else(|| {
                    event
                        .data
                        .pointer("/form/sessionID")
                        .and_then(Value::as_str)
                });
            if event_session != Some(self.session_id.as_str()) {
                continue;
            }
            match event.event.as_deref()? {
                "session.text.delta" => {
                    let delta = event.data.get("delta").and_then(Value::as_str)?;
                    return Some(WorkerEvent::Activity(WorkerActivity::TextDelta {
                        content_index: usize::from(self.reasoning_started),
                        delta: delta.to_owned(),
                    }));
                }
                "session.reasoning.delta" => {
                    let delta = event.data.get("delta").and_then(Value::as_str)?;
                    self.reasoning_started = true;
                    return Some(WorkerEvent::Activity(WorkerActivity::ThinkingDelta {
                        content_index: 0,
                        delta: delta.to_owned(),
                    }));
                }
                "session.tool.input.started" | "session.next.tool.input.started" => {
                    let id = opencode_tool_id(&event.data)?.to_owned();
                    let (name, args) = normalize_opencode_tool(
                        opencode_tool_name(&event.data).unwrap_or("tool"),
                        event.data.get("input").unwrap_or(&Value::Null),
                    );
                    self.active_tools.insert(id.clone(), name.clone());
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolStarted {
                        id,
                        name,
                        args,
                    }));
                }
                "session.tool.called" | "session.next.tool.called" => {
                    let id = opencode_tool_id(&event.data)?.to_owned();
                    let reported_name = opencode_tool_name(&event.data)
                        .or_else(|| self.active_tools.get(&id).map(String::as_str))
                        .unwrap_or("tool");
                    let (name, args) = normalize_opencode_tool(
                        reported_name,
                        event.data.get("input").unwrap_or(&Value::Null),
                    );
                    self.active_tools.insert(id.clone(), name.clone());
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolStarted {
                        id,
                        name,
                        args,
                    }));
                }
                "session.tool.progress" | "session.next.tool.progress" => {
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolUpdated {
                        id: opencode_tool_id(&event.data)?.to_owned(),
                        content: json!([{
                            "type": "text",
                            "text": event.data.get("metadata").map(Value::to_string).unwrap_or_default(),
                        }]),
                    }));
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
                    let turn = TokenUsage {
                        input,
                        output: output.saturating_add(reasoning),
                        cache_read,
                        cache_write,
                    };
                    self.session_usage = self.session_usage.saturating_add(turn);
                    return Some(WorkerEvent::Activity(WorkerActivity::Usage(WorkerUsage {
                        turn,
                        session: self.session_usage,
                        context_window: self.context_window,
                    })));
                }
                "session.tool.success"
                | "session.tool.failed"
                | "session.next.tool.success"
                | "session.next.tool.failed" => {
                    let id = opencode_tool_id(&event.data)?.to_owned();
                    self.active_tools.remove(&id);
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolFinished {
                        id,
                        result: event
                            .data
                            .get("content")
                            .cloned()
                            .unwrap_or_else(|| json!([])),
                        is_error: event
                            .event
                            .as_deref()
                            .is_some_and(|event| event.ends_with("tool.failed")),
                    }));
                }
                "session.compaction.started" | "session.next.compaction.started.1" => {
                    return Some(WorkerEvent::Activity(WorkerActivity::CompactionStarted));
                }
                "session.compaction.ended" | "session.next.compaction.ended.1" => {
                    return Some(WorkerEvent::Activity(WorkerActivity::CompactionFinished {
                        aborted: event
                            .data
                            .get("aborted")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        error: event
                            .data
                            .get("error")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
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
                    self.pending_inputs
                        .insert(id.clone(), PendingOpenCodeInput::Form { key, values });
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
            PendingOpenCodeInput::Permission { session_id } => {
                let reply = opencode_permission_reply(response.value.as_deref(), response.cancel)?;
                client.reply_permission(&session_id, &response.id, reply)
            }
            PendingOpenCodeInput::Form { key, values } => {
                if response.cancel {
                    return client.cancel_form(&self.session_id, &response.id);
                }
                let value = response.value.unwrap_or_default();
                let value = values.get(&value).cloned().unwrap_or(value);
                client.reply_form(&self.session_id, &response.id, json!({key: value}))
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
        self.caller_identity.select_model(provider, model);
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
        self.caller_identity.select_effort(effort);
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
        if let Some(message) = self.caller_identity.try_recv() {
            let activity = if self.completions.is_some() {
                WorkerActivityState::Working
            } else {
                WorkerActivityState::Idle
            };
            let mode = WorkerSendMode::for_peer(activity).expect("OpenCode is ready for delivery");
            return Some(match self.send(message.prompt(), mode) {
                Ok(()) => WorkerEvent::Started,
                Err(error) => WorkerEvent::Failed(error),
            });
        }
        if let Some(event) = self.poll_native_event() {
            return Some(event);
        }
        let completion = self.completions.as_ref()?.try_recv().ok()?;
        if completion.0 != self.generation {
            return None;
        }
        self.completions = None;
        self.caller_identity.set_activity(WorkerActivityState::Idle);
        Some(match completion.1 {
            Ok(output) => WorkerEvent::Settled { output },
            Err(error) => WorkerEvent::Failed(error),
        })
    }

    fn close(&mut self) -> Result<(), String> {
        self.server.terminate()
    }
}

fn opencode_permission_request(event: &super::contract::OpenCodeEvent) -> Option<(&str, &str)> {
    if event.event.as_deref() != Some("permission.asked") {
        return None;
    }
    let session_id = event.data.get("sessionID").and_then(Value::as_str)?;
    let request_id = event.data.get("id").and_then(Value::as_str)?;
    Some((session_id, request_id))
}

fn opencode_permission_prompt(data: &Value) -> String {
    let permission = data
        .get("permission")
        .and_then(Value::as_str)
        .unwrap_or("tool use");
    let patterns = data
        .get("patterns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if patterns.is_empty() {
        format!("OpenCode requests permission for {permission}")
    } else {
        format!(
            "OpenCode requests permission for {permission}\n{}",
            patterns.join("\n")
        )
    }
}

fn opencode_permission_reply(value: Option<&str>, cancel: bool) -> Result<&'static str, String> {
    if cancel {
        return Ok("reject");
    }
    match value.map(str::trim) {
        Some("Allow once") => Ok("once"),
        Some("Always allow") => Ok("always"),
        Some("Decline") => Ok("reject"),
        Some(value) => Err(format!("unknown OpenCode permission response: {value}")),
        None => Err("OpenCode permission response is missing".into()),
    }
}

fn opencode_tool_id(data: &Value) -> Option<&str> {
    data.get("id")
        .and_then(Value::as_str)
        .or_else(|| data.get("callID").and_then(Value::as_str))
}

fn opencode_tool_name(data: &Value) -> Option<&str> {
    data.get("name")
        .and_then(Value::as_str)
        .or_else(|| data.get("tool").and_then(Value::as_str))
}

fn normalize_opencode_tool(name: &str, arguments: &Value) -> (String, Value) {
    let normalized_name = name.trim().to_ascii_lowercase();
    let common = CommonTool::from_name(&normalized_name).or(match normalized_name.as_str() {
        "read_file" => Some(CommonTool::Read),
        "write_file" => Some(CommonTool::Write),
        "edit_file" | "apply_patch" | "patch" => Some(CommonTool::Edit),
        "shell" | "command" | "terminal" => Some(CommonTool::Bash),
        _ => None,
    });
    let Some(common) = common else {
        return (name.to_owned(), arguments.clone());
    };
    let mut normalized = arguments.as_object().cloned().unwrap_or_default();
    rename_argument(&mut normalized, "path", &["file_path", "filePath"]);
    if common == CommonTool::Edit {
        rename_argument(&mut normalized, "oldText", &["old_string", "oldString"]);
        rename_argument(&mut normalized, "newText", &["new_string", "newString"]);
    } else if common == CommonTool::Bash {
        rename_argument(&mut normalized, "command", &["cmd"]);
    }
    (common.name().into(), Value::Object(normalized))
}

fn rename_argument(
    arguments: &mut serde_json::Map<String, Value>,
    canonical: &str,
    aliases: &[&str],
) {
    let value = aliases.iter().find_map(|alias| arguments.remove(*alias));
    if !arguments.contains_key(canonical)
        && let Some(value) = value
    {
        arguments.insert(canonical.into(), value);
    }
    for alias in aliases {
        arguments.remove(*alias);
    }
}

fn start_event_reader(
    server: &OpenCodeServerProcess,
    session_id: &str,
    wake: Option<thread::Thread>,
) -> Result<mpsc::Receiver<Result<super::contract::OpenCodeEvent, String>>, String> {
    let mut stream = server.event_stream()?;
    let (sender, receiver) = mpsc::channel();
    let name = session_id.to_owned();
    thread::Builder::new()
        .name(format!("opencode-events-{name}"))
        .spawn(move || {
            loop {
                let event = match stream.next() {
                    Ok(Some(event)) => Ok(event),
                    Ok(None) => Err("OpenCode event stream closed".into()),
                    Err(error) => Err(error),
                };
                let failed = event.is_err();
                if send_and_wake(&sender, event, wake.as_ref()).is_err() || failed {
                    return;
                }
            }
        })
        .map_err(|error| format!("start OpenCode event reader: {error}"))?;
    Ok(receiver)
}

fn send_and_wake<T>(
    sender: &mpsc::Sender<T>,
    message: T,
    wake: Option<&thread::Thread>,
) -> Result<(), mpsc::SendError<T>> {
    sender.send(message)?;
    if let Some(wake) = wake {
        wake.unpark();
    }
    Ok(())
}

fn configure_opencode_server(
    command: &mut std::process::Command,
    mode: crate::agents::HarnessAccessMode,
) -> Result<(), String> {
    if matches!(mode, crate::agents::HarnessAccessMode::Auto) {
        return Err("OpenCode does not support model-reviewed automatic approvals".into());
    }
    command.args(["serve", "--stdio", "--print-logs"]);
    Ok(())
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
    fn common_tools_use_shared_names_and_arguments() {
        for (source, canonical) in [
            ("read_file", "read"),
            ("write_file", "write"),
            ("apply_patch", "edit"),
            ("shell", "bash"),
        ] {
            assert_eq!(normalize_opencode_tool(source, &json!({})).0, canonical);
        }
        assert_eq!(
            normalize_opencode_tool(
                "edit",
                &json!({"filePath": "src/main.rs", "oldString": "old", "newString": "new"}),
            )
            .1,
            json!({"path": "src/main.rs", "oldText": "old", "newText": "new"})
        );
    }

    #[test]
    fn permission_requests_keep_child_session_identity() {
        let event = super::super::contract::OpenCodeEvent {
            id: None,
            event: Some("permission.asked".into()),
            data: json!({"sessionID": "child-1", "id": "permission-1"}),
        };

        assert_eq!(
            opencode_permission_request(&event),
            Some(("child-1", "permission-1"))
        );
    }

    #[test]
    fn supported_modes_use_the_opencode_server_without_auto_approval() {
        for mode in [
            crate::agents::HarnessAccessMode::Sandboxed,
            crate::agents::HarnessAccessMode::Full,
        ] {
            let mut command = std::process::Command::new("opencode2");
            configure_opencode_server(&mut command, mode).expect("supported OpenCode mode");
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                ["serve", "--stdio", "--print-logs"]
            );
        }

        let mut command = std::process::Command::new("opencode2");
        assert_eq!(
            configure_opencode_server(&mut command, crate::agents::HarnessAccessMode::Auto),
            Err("OpenCode does not support model-reviewed automatic approvals".into())
        );
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn sandboxed_permission_requests_keep_native_choices() {
        let data = json!({
            "permission": "bash",
            "patterns": ["git status", "git diff"]
        });
        assert_eq!(
            opencode_permission_prompt(&data),
            "OpenCode requests permission for bash\ngit status\ngit diff"
        );
        assert_eq!(
            opencode_permission_reply(Some("Allow once"), false),
            Ok("once")
        );
        assert_eq!(
            opencode_permission_reply(Some("Always allow"), false),
            Ok("always")
        );
        assert_eq!(
            opencode_permission_reply(Some("Decline"), false),
            Ok("reject")
        );
        assert_eq!(opencode_permission_reply(None, true), Ok("reject"));
    }

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
