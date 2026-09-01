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
    agents::{
        AgentLaunchConfig, CommonTool, TokenUsage, WorkerActivity, WorkerContext, WorkerEvent,
        WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession,
        WorkerSessionFactory, WorkerUsage,
    },
    modules::agents::adapter::{child_stderr, farcaster_mcp, main_session},
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
        let mut prepared = self.command.command(&launch.project)?;
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue_as(
            &launch.project,
            crate::modules::agents::core::CallerProfile {
                backend: "codex-cli".into(),
                provider: launch.provider.clone(),
                model: launch.model.clone(),
                effort: launch.effort.clone(),
            },
            None,
            launch.worker_id.clone(),
        );
        configure_codex_app_server(&mut prepared, self.command.access_mode);
        if farcaster_mcp::enabled() {
            configure_farcaster_mcp(&mut prepared, caller_identity.token());
        }
        let mut child = prepared
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start Codex worker app-server: {error}"))?;
        child_stderr::capture(&mut child, "codex-worker")?;
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
            caller_identity,
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
            reasoning_started: false,
            compacting: false,
            manual_compaction: false,
            pending: HashMap::new(),
            pending_inputs: HashMap::new(),
            events: VecDeque::from([WorkerEvent::SessionChanged { locator: thread_id }]),
        }))
    }
}

pub(in crate::modules::agents::adapter) fn load_configuration(
    command: &AgentLaunchConfig,
    project: &std::path::Path,
) -> Result<crate::modules::agents::adapter::main_session::MainSessionMetadata, String> {
    let mut prepared = command.command(project)?;
    configure_codex_app_server(&mut prepared, command.access_mode);
    let mut child = prepared
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start Codex catalog app-server: {error}"))?;
    child_stderr::capture(&mut child, "codex-catalog")?;
    let result = (|| {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex catalog stdin must be piped".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex catalog stdout must be piped".to_owned())?;
        let mut connection = CodexConnection::new(BufReader::new(stdout), stdin);
        connection.initialize_experimental(CodexClientInfo {
            name: "farcaster".into(),
            title: Some("Farcaster".into()),
            version: env!("CARGO_PKG_VERSION").into(),
        })?;
        load_main_metadata(&mut connection)
    })();
    let _ = child.kill();
    let _ = child.wait();
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
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
    );
    configure_codex_app_server(&mut prepared, command.access_mode);
    if farcaster_mcp::enabled() {
        configure_farcaster_mcp(&mut prepared, caller_identity.token());
    }
    let mut child = prepared
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start Codex main-session app-server: {error}"))?;
    child_stderr::capture(&mut child, "codex-main-session")?;
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
    let wake = launch.wake.clone();
    thread::Builder::new()
        .name(format!("codex-session-{reader_name}"))
        .spawn(move || {
            for message in queued {
                if send_and_wake(&sender, Ok(message), wake.as_ref()).is_err() {
                    return;
                }
            }
            loop {
                let message = read_message(&mut reader);
                let failed = message.is_err();
                if send_and_wake(&sender, message, wake.as_ref()).is_err() || failed {
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
        caller_identity,
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
        reasoning_started: false,
        compacting: false,
        manual_compaction: false,
        pending: HashMap::new(),
        pending_inputs: HashMap::new(),
        events: VecDeque::new(),
    };
    Ok((Box::new(session), thread_id, metadata))
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
        crate::agents::SessionStart::Resume(_) => {
            let thread_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "Codex resume requires a thread id".to_owned())?;
            connection.resume_thread(&thread_id)?
        }
        crate::agents::SessionStart::Fork(_) => {
            let thread_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "Codex fork requires a thread id".to_owned())?;
            connection.fork_thread(&thread_id, &cwd, None, None)?
        }
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
    Ok(
        crate::modules::agents::adapter::main_session::MainSessionMetadata {
            models,
            efforts,
            commands: Vec::new(),
            modes,
        },
    )
}

#[derive(Clone, Copy)]
enum PendingRequest {
    StartTurn,
    Ignore,
}

struct CodexWorkerSession {
    caller_identity: crate::modules::agents::core::CallerIdentity,
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
    reasoning_started: bool,
    compacting: bool,
    manual_compaction: bool,
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
        let id = self.request("thread/compact/start", json!({"threadId": self.thread_id}))?;
        self.manual_compaction = true;
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

    fn select_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        self.caller_identity.select_model(provider, model);
        self.model = Some(model.to_owned());
        Ok(())
    }

    fn select_effort(&mut self, effort: &str) -> Result<(), String> {
        self.caller_identity.select_effort(effort);
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
        if let Some(message) = self.caller_identity.try_recv() {
            let mode = WorkerSendMode::for_peer(self.current_turn.is_some());
            return Some(match self.send(message.prompt(), mode) {
                Ok(()) => WorkerEvent::Started,
                Err(error) => WorkerEvent::Failed(error),
            });
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
                                    self.reasoning_started = false;
                                    return Some(WorkerEvent::Started);
                                }
                            }
                        }
                        "item/agentMessage/delta" => {
                            if let Some(delta) = params["delta"].as_str() {
                                self.output.push_str(delta);
                                return Some(WorkerEvent::Activity(WorkerActivity::TextDelta {
                                    content_index: usize::from(self.reasoning_started),
                                    delta: delta.to_owned(),
                                }));
                            }
                        }
                        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                            if let Some(delta) = params["delta"].as_str() {
                                self.reasoning_started = true;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::ThinkingDelta {
                                        content_index: 0,
                                        delta: delta.to_owned(),
                                    },
                                ));
                            }
                        }
                        "item/started" => {
                            if params.pointer("/item/type").and_then(Value::as_str)
                                == Some("contextCompaction")
                            {
                                self.compacting = true;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::CompactionStarted,
                                ));
                            }
                            if let Some(event) = codex_tool_start(&params) {
                                return Some(WorkerEvent::Activity(event));
                            }
                        }
                        "item/commandExecution/outputDelta" => {
                            if let (Some(id), Some(delta)) =
                                (params["itemId"].as_str(), params["delta"].as_str())
                            {
                                return Some(WorkerEvent::Activity(WorkerActivity::ToolUpdated {
                                    id: id.to_owned(),
                                    content: json!([{"type": "text", "text": delta}]),
                                }));
                            }
                        }
                        "item/completed" => {
                            if self.output.is_empty()
                                && let Some(output) = codex_agent_message_text(&params["item"])
                            {
                                self.output.push_str(&output);
                            }
                            if params.pointer("/item/type").and_then(Value::as_str)
                                == Some("contextCompaction")
                            {
                                self.compacting = false;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::CompactionFinished {
                                        aborted: false,
                                        error: None,
                                    },
                                ));
                            }
                            if params.pointer("/item/type").and_then(Value::as_str)
                                == Some("webSearch")
                                && let Some(started) = codex_tool_start(&params)
                                && let Some(finished) = codex_tool_end(&params)
                            {
                                self.events.push_back(WorkerEvent::Activity(finished));
                                return Some(WorkerEvent::Activity(started));
                            }
                            if let Some(event) = codex_tool_end(&params) {
                                return Some(WorkerEvent::Activity(event));
                            }
                        }
                        "thread/tokenUsage/updated" => {
                            let usage = params.get("tokenUsage")?;
                            let session = codex_usage(usage.get("total")?);
                            let turn = usage.get("last").map(codex_usage).unwrap_or(session);
                            return Some(WorkerEvent::Activity(WorkerActivity::Usage(
                                WorkerUsage {
                                    turn,
                                    session,
                                    context_window: usage
                                        .get("modelContextWindow")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0),
                                },
                            )));
                        }
                        "turn/completed" => {
                            self.current_turn = None;
                            let failed = params["turn"]["status"].as_str() == Some("failed");
                            if self.manual_compaction {
                                self.manual_compaction = false;
                                if self.compacting || failed {
                                    self.compacting = false;
                                    self.events.push_back(WorkerEvent::Settled {
                                        output: String::new(),
                                    });
                                    return Some(WorkerEvent::Activity(
                                        WorkerActivity::CompactionFinished {
                                            aborted: false,
                                            error: failed.then(|| "Codex compaction failed".into()),
                                        },
                                    ));
                                }
                                return Some(WorkerEvent::Settled {
                                    output: String::new(),
                                });
                            }
                            if failed {
                                return Some(WorkerEvent::Failed(
                                    "Codex worker turn failed".into(),
                                ));
                            }
                            return Some(WorkerEvent::Settled {
                                output: self.output.clone(),
                            });
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

fn configure_codex_app_server(
    command: &mut std::process::Command,
    mode: crate::agents::HarnessAccessMode,
) {
    super::configure_permissions(command, mode);
    command.args(["app-server", "--stdio", "--enable", "mcp_2026_07_28"]);
}

fn configure_farcaster_mcp(command: &mut std::process::Command, caller_token: &str) {
    let url = serde_json::to_string(farcaster_mcp::URL).expect("static MCP URL encodes");
    let header =
        serde_json::to_string(farcaster_mcp::CALLER_HEADER).expect("static MCP header encodes");
    let token = serde_json::to_string(caller_token).expect("caller token encodes");
    command
        .arg("-c")
        .arg(format!("mcp_servers.farcaster.url={url}"))
        .arg("-c")
        .arg(format!(
            "mcp_servers.farcaster.http_headers={{{header}={token}}}"
        ))
        .arg("-c")
        .arg("mcp_servers.farcaster.required=true");
}

fn codex_agent_message_text(item: &Value) -> Option<String> {
    if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
        return None;
    }
    item.get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let content = item.get("content")?.as_array()?;
            Some(
                content
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect(),
            )
        })
}

fn codex_usage(value: &Value) -> TokenUsage {
    let input = value
        .get("inputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = value
        .get("cachedInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = value
        .get("cacheWriteInputTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    TokenUsage {
        // Codex reports cached tokens as part of inputTokens. The shared
        // contract keeps input, cache reads, and cache writes disjoint.
        input: input.saturating_sub(cache_read.saturating_add(cache_write)),
        output: value
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read,
        cache_write,
    }
}

fn codex_tool_start(params: &Value) -> Option<WorkerActivity> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !matches!(
        kind,
        "commandExecution" | "mcpToolCall" | "fileChange" | "webSearch"
    ) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let (name, args) = codex_tool_call(item, kind);
    Some(WorkerActivity::ToolStarted {
        id: id.to_owned(),
        name,
        args,
    })
}

fn codex_tool_call(item: &Value, kind: &str) -> (String, Value) {
    match kind {
        "fileChange" => {
            let changes = item.get("changes").cloned().unwrap_or_else(|| json!([]));
            let path = changes
                .as_array()
                .and_then(|changes| changes.first())
                .and_then(|change| change.get("path"))
                .cloned()
                .unwrap_or(Value::String(String::new()));
            (
                CommonTool::Edit.name().into(),
                json!({"path": path, "changes": changes}),
            )
        }
        "commandExecution" => {
            let read = item
                .get("commandActions")
                .and_then(Value::as_array)
                .filter(|actions| actions.len() == 1)
                .and_then(|actions| actions.first())
                .filter(|action| action.get("type").and_then(Value::as_str) == Some("read"));
            if let Some(action) = read {
                (
                    CommonTool::Read.name().into(),
                    json!({"path": action.get("path").cloned().unwrap_or(Value::Null)}),
                )
            } else {
                (
                    CommonTool::Bash.name().into(),
                    json!({"command": item.get("command").cloned().unwrap_or(Value::Null)}),
                )
            }
        }
        "webSearch" => (
            "web_search".into(),
            json!({"query": codex_web_search_query(item)}),
        ),
        _ => {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| item.get("server").and_then(Value::as_str))
                .unwrap_or(kind);
            let args = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
            (name.to_owned(), args)
        }
    }
}

fn codex_web_search_query(item: &Value) -> Option<&str> {
    item.get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.is_empty())
        .or_else(|| {
            let action = item.get("action")?;
            ["query", "url", "pattern"]
                .into_iter()
                .find_map(|field| action.get(field).and_then(Value::as_str))
        })
}

fn codex_tool_end(params: &Value) -> Option<WorkerActivity> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !matches!(
        kind,
        "commandExecution" | "mcpToolCall" | "fileChange" | "webSearch"
    ) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let failed = item
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "failed" | "declined"));
    let output = item
        .get("aggregatedOutput")
        .and_then(Value::as_str)
        .or_else(|| item.get("result").and_then(Value::as_str))
        .or_else(|| {
            (kind == "webSearch")
                .then(|| codex_web_search_query(item))
                .flatten()
        })
        .unwrap_or_else(|| {
            if kind == "fileChange" && !failed {
                "Applied patch"
            } else {
                ""
            }
        });
    Some(WorkerActivity::ToolFinished {
        id: id.to_owned(),
        result: json!([{"type": "text", "text": output}]),
        is_error: failed,
    })
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
    fn extracts_completed_agent_message_text() {
        assert_eq!(
            codex_agent_message_text(&json!({
                "type": "agentMessage",
                "content": [{"type": "Text", "text": "hello"}]
            }))
            .as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn codex_tools_use_shared_names_and_arguments() {
        let change = json!({"changes":[{"path":"src/main.rs","diff":"-old\n+new\n"}]});
        assert_eq!(
            codex_tool_call(&change, "fileChange"),
            (
                "edit".into(),
                json!({"path":"src/main.rs","changes":change["changes"]})
            )
        );

        let read = json!({
            "command":"cat src/main.rs",
            "commandActions":[{"type":"read","path":"src/main.rs"}]
        });
        assert_eq!(
            codex_tool_call(&read, "commandExecution"),
            ("read".into(), json!({"path":"src/main.rs"}))
        );
        assert_eq!(
            codex_tool_call(&json!({"command":"cargo test"}), "commandExecution"),
            ("bash".into(), json!({"command":"cargo test"}))
        );
        assert_eq!(
            codex_tool_call(&json!({"query":"Codex app-server protocol"}), "webSearch"),
            (
                "web_search".into(),
                json!({"query":"Codex app-server protocol"})
            )
        );
    }

    #[test]
    fn completed_web_search_exposes_its_query_as_output() {
        let event = codex_tool_end(&json!({
            "item": {
                "id": "search-1",
                "type": "webSearch",
                "query": "Codex app-server protocol"
            }
        }));
        assert_eq!(
            event,
            Some(WorkerActivity::ToolFinished {
                id: "search-1".into(),
                result: json!([{"type":"text", "text":"Codex app-server protocol"}]),
                is_error: false,
            })
        );
    }

    #[test]
    fn codex_usage_separates_cached_tokens_from_reported_input() {
        assert_eq!(
            codex_usage(&json!({
                "inputTokens": 1_000,
                "outputTokens": 50,
                "cachedInputTokens": 950,
                "cacheWriteInputTokens": 0
            })),
            TokenUsage {
                input: 50,
                output: 50,
                cache_read: 950,
                cache_write: 0,
            }
        );
    }

    #[test]
    fn native_startup_configures_required_farcaster_mcp() {
        let mut command = std::process::Command::new("codex");
        configure_codex_app_server(&mut command, crate::agents::HarnessAccessMode::Full);
        configure_farcaster_mcp(&mut command, "caller-1");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &arguments[..5],
            [
                "--dangerously-bypass-approvals-and-sandbox",
                "app-server",
                "--stdio",
                "--enable",
                "mcp_2026_07_28",
            ]
        );
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
