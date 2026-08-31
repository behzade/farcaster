use std::{
    collections::{HashMap, VecDeque},
    io::{BufReader, Write as _},
    process::{Child, ChildStdin, Stdio},
    sync::mpsc,
    thread,
};

use serde_json::{Value, json};

use super::{
    connection::{CodexConnection, read_message},
    contract::{CodexClientInfo, CodexInbound, CodexRequestId, CodexUserInput, TurnResponse},
    wire::{encode_request, encode_response},
};
use crate::{
    access::SandboxedCommand,
    agents::{
        AgentLaunchConfig, WorkerContext, WorkerEvent, WorkerInput, WorkerInputResponse,
        WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
    },
    modules::agents::adapter::farcaster_mcp,
};

#[derive(Clone)]
pub(crate) struct CodexWorkerFactory {
    command: AgentLaunchConfig,
}

impl CodexWorkerFactory {
    pub(crate) fn new(command: AgentLaunchConfig) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for CodexWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("Codex worker provider and model must be supplied together".into());
        }
        let mut sandbox = self.command.command(&launch.project)?;
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue();
        configure_farcaster_mcp(&mut sandbox.command, caller_identity.token());
        let mut child = sandbox
            .command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("start Codex worker app-server: {error}"))?;
        let (mut reader, writer, queued, next_id, thread) =
            match setup_connection(&mut child, &launch) {
                Ok(setup) => setup,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            };
        let (sender, incoming) = mpsc::channel();
        if let Err(error) = thread::Builder::new()
            .name(format!("codex-worker-{}", thread.id))
            .spawn(move || {
                for message in queued {
                    if sender.send(Ok(message)).is_err() {
                        return;
                    }
                }
                loop {
                    let message = read_message(&mut reader);
                    let failed = message.is_err();
                    if sender.send(message).is_err() || failed {
                        return;
                    }
                }
            })
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("read Codex worker events: {error}"));
        }
        let thread_id = thread.id;
        caller_identity.bind(thread_id.clone());
        Ok(Box::new(CodexWorkerSession {
            _caller_identity: caller_identity,
            _sandbox: sandbox,
            child,
            writer,
            incoming,
            thread_id: thread_id.clone(),
            model: launch.model,
            effort: launch.effort,
            collaboration_mode: None,
            collaboration_modes: HashMap::new(),
            native_queue: false,
            next_id,
            current_turn: None,
            output: String::new(),
            message_started: false,
            reasoning_started: false,
            pending: HashMap::new(),
            pending_inputs: HashMap::new(),
            events: VecDeque::from([WorkerEvent::SessionChanged { locator: thread_id }]),
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
    let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue();
    configure_farcaster_mcp(&mut sandbox.command, caller_identity.token());
    let mut child = sandbox
        .command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("start Codex main-session app-server: {error}"))?;
    let setup = setup_main_connection(&mut child, launch);
    let ((mut reader, writer, queued, next_id, thread), metadata) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let (sender, incoming) = mpsc::channel();
    let thread_id = thread.id.clone();
    let reader_name = thread_id.clone();
    thread::Builder::new()
        .name(format!("codex-session-{reader_name}"))
        .spawn(move || {
            for message in queued {
                if sender.send(Ok(message)).is_err() {
                    return;
                }
            }
            loop {
                let message = read_message(&mut reader);
                let failed = message.is_err();
                if sender.send(message).is_err() || failed {
                    return;
                }
            }
        })
        .map_err(|error| format!("read Codex main-session events: {error}"))?;
    caller_identity.bind(thread_id.clone());
    let collaboration_modes = metadata
        .modes
        .iter()
        .filter_map(|mode| {
            Some((
                mode.get("id")?.as_str()?.to_owned(),
                mode.get("configuration")?.clone(),
            ))
        })
        .collect();
    let session = CodexWorkerSession {
        _caller_identity: caller_identity,
        _sandbox: sandbox,
        child,
        writer,
        incoming,
        thread_id: thread_id.clone(),
        model: None,
        effort: None,
        collaboration_mode: None,
        collaboration_modes,
        native_queue: true,
        next_id,
        current_turn: None,
        output: String::new(),
        message_started: false,
        reasoning_started: false,
        pending: HashMap::new(),
        pending_inputs: HashMap::new(),
        events: VecDeque::new(),
    };
    Ok((Box::new(session), thread_id, metadata))
}

type CodexSetup = (
    BufReader<std::process::ChildStdout>,
    ChildStdin,
    VecDeque<CodexInbound>,
    i64,
    super::contract::CodexThread,
);

fn setup_connection(child: &mut Child, launch: &WorkerLaunch) -> Result<CodexSetup, String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex worker stdin must be piped".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex worker stdout must be piped".to_owned())?;
    let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
    connection.initialize(CodexClientInfo {
        name: "farcaster".into(),
        title: Some("Farcaster".into()),
        version: env!("CARGO_PKG_VERSION").into(),
    })?;
    let cwd = launch.project.to_string_lossy();
    let thread = match &launch.context {
        WorkerContext::Fresh => {
            connection.start_thread(&cwd, launch.provider.as_deref(), launch.model.as_deref())?
        }
        WorkerContext::Session { session_locator } => {
            if session_locator != &launch.parent_session {
                return Err(
                    "Codex workers cannot inherit context from a thread other than their parent"
                        .into(),
                );
            }
            connection.fork_thread(
                session_locator,
                &cwd,
                launch.provider.as_deref(),
                launch.model.as_deref(),
            )?
        }
    };
    let (reader, writer, queued, next_id) = connection.into_parts();
    Ok((reader, writer, queued, next_id, thread))
}

fn setup_main_connection(
    child: &mut Child,
    launch: &crate::agents::SessionLaunch,
) -> Result<
    (
        CodexSetup,
        crate::modules::agents::adapter::main_session::MainSessionMetadata,
    ),
    String,
> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex main-session stdin must be piped".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex main-session stdout must be piped".to_owned())?;
    let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
    connection.initialize_experimental(CodexClientInfo {
        name: "farcaster".into(),
        title: Some("Farcaster".into()),
        version: env!("CARGO_PKG_VERSION").into(),
    })?;
    let metadata = load_main_metadata(&mut connection)?;
    let cwd = launch.project.to_string_lossy();
    let thread = match &launch.start {
        crate::agents::SessionStart::New => connection.start_thread(&cwd, None, None)?,
        crate::agents::SessionStart::Resume(_) => connection.resume_thread(
            launch
                .session_id
                .as_deref()
                .ok_or_else(|| "Codex resume requires a thread id".to_owned())?,
        )?,
        crate::agents::SessionStart::Fork(_) => connection.fork_thread(
            launch
                .session_id
                .as_deref()
                .ok_or_else(|| "Codex fork requires a thread id".to_owned())?,
            &cwd,
            None,
            None,
        )?,
    };
    let (reader, writer, queued, next_id) = connection.into_parts();
    Ok(((reader, writer, queued, next_id, thread), metadata))
}

fn load_main_metadata(
    connection: &mut CodexConnection<BufReader<std::process::ChildStdout>, ChildStdin>,
) -> Result<crate::modules::agents::adapter::main_session::MainSessionMetadata, String> {
    let id = connection.send_request("model/list", json!({"limit": 100}))?;
    let response: Value = connection.wait_response(&id)?;
    let mut efforts = Vec::new();
    let models: Vec<Value> = response
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?;
            for effort in model
                .get("supportedReasoningEfforts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let effort = effort
                    .as_str()
                    .or_else(|| effort.get("reasoningEffort")?.as_str());
                if let Some(effort) = effort
                    && !efforts.iter().any(|known| known == effort)
                {
                    efforts.push(effort.to_owned());
                }
            }
            Some(json!({
                "id": id,
                "name": model
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(id),
                "provider": model
                    .get("modelProvider")
                    .and_then(Value::as_str)
                    .unwrap_or("openai"),
                "contextWindow": model
                    .get("contextWindow")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                "reasoning": model.get("supportedReasoningEfforts").is_some(),
            }))
        })
        .collect();
    let default_model = models
        .first()
        .and_then(|model| model.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = connection.send_request("collaborationMode/list", json!({}))?;
    let response: Value = connection.wait_response(&id)?;
    let modes = response
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|mode| {
            let id = mode.get("mode")?.as_str()?;
            let model = mode
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(default_model);
            let effort = mode.get("reasoning_effort").cloned().unwrap_or(Value::Null);
            Some(json!({
                "id": id,
                "name": mode.get("name").and_then(Value::as_str).unwrap_or(id),
                "description": null,
                "configuration": {
                    "mode": id,
                    "settings": {
                        "model": model,
                        "reasoning_effort": effort,
                        "developer_instructions": null,
                    }
                }
            }))
        })
        .collect();
    Ok(crate::modules::agents::adapter::main_session::MainSessionMetadata {
        models,
        efforts,
        commands: Vec::new(),
        modes,
    })
}

#[derive(Clone, Copy)]
enum PendingRequest {
    StartTurn,
    Ignore,
}

struct CodexWorkerSession {
    _caller_identity: crate::modules::agents::core::CallerIdentity,
    _sandbox: SandboxedCommand,
    child: Child,
    writer: ChildStdin,
    incoming: mpsc::Receiver<Result<CodexInbound, String>>,
    thread_id: String,
    model: Option<String>,
    effort: Option<String>,
    collaboration_mode: Option<Value>,
    collaboration_modes: HashMap<String, Value>,
    native_queue: bool,
    next_id: i64,
    current_turn: Option<String>,
    output: String,
    message_started: bool,
    reasoning_started: bool,
    pending: HashMap<CodexRequestId, PendingRequest>,
    pending_inputs: HashMap<String, CodexRequestId>,
    events: VecDeque<WorkerEvent>,
}

impl WorkerSession for CodexWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_input(vec![CodexUserInput::text(message)], mode)
    }

    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        let mut input = vec![CodexUserInput::text(message)];
        input.extend(images.into_iter().map(|image| CodexUserInput::Image {
            url: format!("data:{};base64,{}", image.mime_type, image.data),
        }));
        self.send_input(input, mode)
    }

    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        let request_id = self
            .pending_inputs
            .remove(&response.id)
            .ok_or_else(|| format!("unknown Codex worker input: {}", response.id))?;
        let accepted = !response.cancel
            && response.value.as_deref().is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "yes" | "true" | "allow" | "accept" | "accepted"
                )
            });
        let result = json!({"decision": if accepted { "accept" } else { "decline" }});
        self.writer
            .write_all(&encode_response(&request_id, result)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("answer Codex worker request: {error}"))
    }

    fn abort(&mut self) -> Result<(), String> {
        let Some(turn_id) = self.current_turn.clone() else {
            return Ok(());
        };
        let id = self.request(
            "turn/interrupt",
            json!({"threadId": self.thread_id, "turnId": turn_id}),
        )?;
        self.pending.insert(id, PendingRequest::Ignore);
        Ok(())
    }

    fn compact(&mut self) -> Result<(), String> {
        let id = self.request(
            "thread/compact/start",
            json!({"threadId": self.thread_id}),
        )?;
        self.pending.insert(id, PendingRequest::Ignore);
        Ok(())
    }

    fn rename(&mut self, name: &str) -> Result<(), String> {
        let id = self.request(
            "thread/name/set",
            json!({"threadId": self.thread_id, "name": name}),
        )?;
        self.pending.insert(id, PendingRequest::Ignore);
        Ok(())
    }

    fn select_model(&mut self, _provider: &str, model: &str) -> Result<(), String> {
        self.model = Some(model.to_owned());
        Ok(())
    }

    fn select_effort(&mut self, effort: &str) -> Result<(), String> {
        self.effort = Some(effort.to_owned());
        Ok(())
    }

    fn select_mode(&mut self, mode: &str) -> Result<(), String> {
        self.collaboration_mode = Some(
            self.collaboration_modes
                .get(mode)
                .cloned()
                .ok_or_else(|| format!("unknown Codex collaboration mode: {mode}"))?,
        );
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.events.pop_front() {
            return Some(event);
        }
        loop {
            match self.incoming.try_recv().ok()? {
                Ok(CodexInbound::Response { id, result }) => match self.pending.remove(&id) {
                    Some(PendingRequest::StartTurn) => {
                        let turn = match serde_json::from_value::<TurnResponse>(result) {
                            Ok(response) => response.turn,
                            Err(error) => {
                                return Some(WorkerEvent::Failed(format!(
                                    "decode Codex worker turn: {error}"
                                )));
                            }
                        };
                        self.current_turn = Some(turn.id);
                        return Some(WorkerEvent::Started);
                    }
                    Some(PendingRequest::Ignore) | None => {}
                },
                Ok(CodexInbound::Error { id, error }) => {
                    self.pending.remove(&id);
                    return Some(WorkerEvent::Failed(format!(
                        "Codex app-server error {}: {}",
                        error.code, error.message
                    )));
                }
                Ok(CodexInbound::Notification { method, params }) => {
                    if params["threadId"].as_str() != Some(&self.thread_id) {
                        continue;
                    }
                    match method.as_str() {
                        "turn/started" => {
                            if let Some(turn_id) = params["turn"]["id"].as_str() {
                                let new_turn = self.current_turn.as_deref() != Some(turn_id);
                                self.current_turn = Some(turn_id.to_owned());
                                if new_turn {
                                    self.output.clear();
                                    self.message_started = false;
                                    self.reasoning_started = false;
                                    return Some(WorkerEvent::Started);
                                }
                            }
                        }
                        "item/agentMessage/delta" => {
                            if let Some(delta) = params["delta"].as_str() {
                                self.output.push_str(delta);
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
                                    self.events.push_back(update);
                                    return Some(WorkerEvent::Activity(json!({
                                        "type": "message_start",
                                        "message": {"role": "assistant", "content": []}
                                    })));
                                }
                                return Some(update);
                            }
                        }
                        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                            if let Some(delta) = params["delta"].as_str() {
                                if !self.message_started {
                                    self.message_started = true;
                                    self.events.push_back(WorkerEvent::Activity(json!({
                                        "type": "message_update",
                                        "assistantMessageEvent": {
                                            "type": "thinking_start",
                                            "contentIndex": 0,
                                        }
                                    })));
                                    self.events.push_back(WorkerEvent::Activity(json!({
                                        "type": "message_update",
                                        "assistantMessageEvent": {
                                            "type": "thinking_delta",
                                            "contentIndex": 0,
                                            "delta": delta,
                                        }
                                    })));
                                    self.reasoning_started = true;
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
                        }
                        "item/started" => {
                            if let Some(event) = codex_tool_start(&params) {
                                return Some(WorkerEvent::Activity(event));
                            }
                        }
                        "item/commandExecution/outputDelta" => {
                            if let (Some(id), Some(delta)) =
                                (params["itemId"].as_str(), params["delta"].as_str())
                            {
                                return Some(WorkerEvent::Activity(json!({
                                    "type": "tool_execution_update",
                                    "toolCallId": id,
                                    "partialResult": {"content": [{"type": "text", "text": delta}]},
                                })));
                            }
                        }
                        "item/completed" => {
                            if let Some(event) = codex_tool_end(&params) {
                                return Some(WorkerEvent::Activity(event));
                            }
                        }
                        "turn/completed" => {
                            self.current_turn = None;
                            if params["turn"]["status"].as_str() == Some("failed") {
                                return Some(WorkerEvent::Failed(
                                    "Codex worker turn failed".into(),
                                ));
                            }
                            let settled = WorkerEvent::Settled {
                                output: self.output.clone(),
                            };
                            if self.message_started {
                                self.events.push_back(settled);
                                return Some(WorkerEvent::Activity(json!({
                                    "type": "message_end",
                                    "message": {"role": "assistant"}
                                })));
                            }
                            return Some(settled);
                        }
                        _ => {}
                    }
                }
                Ok(CodexInbound::ServerRequest { id, method, params }) => {
                    let input_id = match &id {
                        CodexRequestId::Number(value) => value.to_string(),
                        CodexRequestId::String(value) => value.clone(),
                    };
                    self.pending_inputs.insert(input_id.clone(), id);
                    return Some(WorkerEvent::NeedsInput(WorkerInput {
                        id: input_id,
                        prompt: approval_prompt(&method, &params),
                        options: vec!["Allow".into(), "Decline".into()],
                        secret: false,
                    }));
                }
                Err(error) => return Some(WorkerEvent::Failed(error)),
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|error| format!("check Codex worker: {error}"))?
            .is_none()
        {
            self.child
                .kill()
                .map_err(|error| format!("terminate Codex worker: {error}"))?;
        }
        self.child
            .wait()
            .map_err(|error| format!("reap Codex worker: {error}"))?;
        Ok(())
    }
}

impl CodexWorkerSession {
    fn send_input(
        &mut self,
        input: Vec<CodexUserInput>,
        mode: WorkerSendMode,
    ) -> Result<(), String> {
        if mode == WorkerSendMode::Steer {
            let turn_id = self
                .current_turn
                .as_deref()
                .ok_or_else(|| "Codex worker has not reported its active turn".to_owned())?;
            let id = self.request(
                "turn/steer",
                json!({
                    "threadId": self.thread_id,
                    "expectedTurnId": turn_id,
                    "input": input,
                }),
            )?;
            self.pending.insert(id, PendingRequest::Ignore);
            return Ok(());
        }
        if mode == WorkerSendMode::Queue && self.native_queue {
            let client_id = format!("farcaster-queue-{}", self.next_id.saturating_add(1));
            let id = self.request(
                "thread/queue/add",
                json!({
                    "threadId": self.thread_id,
                    "clientUserMessageId": client_id,
                    "input": input,
                }),
            )?;
            self.pending.insert(id, PendingRequest::Ignore);
            return Ok(());
        }
        self.output.clear();
        self.message_started = false;
        self.reasoning_started = false;
        let id = self.request(
            "turn/start",
            json!({
                "threadId": self.thread_id,
                "input": input,
                "model": self.model,
                "effort": self.effort,
                "collaborationMode": self.collaboration_mode,
            }),
        )?;
        self.pending.insert(id, PendingRequest::StartTurn);
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<CodexRequestId, String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "Codex worker request id overflow".to_owned())?;
        let id = CodexRequestId::Number(self.next_id);
        self.writer
            .write_all(&encode_request(&id, method, params)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("write Codex worker request: {error}"))?;
        Ok(id)
    }
}

impl Drop for CodexWorkerSession {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn configure_farcaster_mcp(command: &mut std::process::Command, caller_token: &str) {
    let url = serde_json::to_string(farcaster_mcp::URL).expect("static MCP URL encodes");
    let header =
        serde_json::to_string(farcaster_mcp::CALLER_HEADER).expect("static MCP header encodes");
    let token = serde_json::to_string(caller_token).expect("caller token encodes");
    command
        .args(["app-server", "--stdio"])
        .arg("-c")
        .arg(format!("mcp_servers.farcaster.url={url}"))
        .arg("-c")
        .arg(format!(
            "mcp_servers.farcaster.http_headers={{{header}={token}}}"
        ))
        .arg("-c")
        .arg("mcp_servers.farcaster.required=true");
}

fn codex_tool_start(params: &Value) -> Option<Value> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !matches!(
        kind,
        "commandExecution" | "mcpToolCall" | "fileChange" | "webSearch"
    ) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let name = item
        .get("server")
        .and_then(Value::as_str)
        .or_else(|| item.get("name").and_then(Value::as_str))
        .unwrap_or(kind);
    let args = item
        .get("arguments")
        .cloned()
        .or_else(|| item.get("command").cloned())
        .unwrap_or_else(|| json!({}));
    Some(json!({
        "type": "tool_execution_start",
        "toolCallId": id,
        "toolName": name,
        "args": args,
    }))
}

fn codex_tool_end(params: &Value) -> Option<Value> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !matches!(
        kind,
        "commandExecution" | "mcpToolCall" | "fileChange" | "webSearch"
    ) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let output = item
        .get("aggregatedOutput")
        .and_then(Value::as_str)
        .or_else(|| item.get("result").and_then(Value::as_str))
        .unwrap_or_default();
    let failed = item
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "failed" | "declined"));
    Some(json!({
        "type": "tool_execution_end",
        "toolCallId": id,
        "result": {"content": [{"type": "text", "text": output}]},
        "isError": failed,
    }))
}

fn approval_prompt(method: &str, params: &Value) -> String {
    params["command"]
        .as_str()
        .or_else(|| params["reason"].as_str())
        .map_or_else(|| method.to_owned(), |detail| format!("{method}\n{detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_startup_configures_required_farcaster_mcp() {
        let mut command = std::process::Command::new("codex");
        configure_farcaster_mcp(&mut command, "caller-1");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(&arguments[..2], ["app-server", "--stdio"]);
        assert!(arguments.contains(&format!(
            "mcp_servers.farcaster.url=\"{}\"",
            farcaster_mcp::URL
        )));
        assert!(arguments.contains(
            &"mcp_servers.farcaster.http_headers={\"farcaster-caller\"=\"caller-1\"}".to_owned()
        ));
        assert!(arguments.contains(&"mcp_servers.farcaster.required=true".to_owned()));
    }
}
