#[path = "commands.rs"]
mod commands;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufReader, Write as _},
    process::{Child, ChildStdin, Stdio},
    sync::mpsc,
    thread,
};

use serde_json::{Value, json};

use super::{
    connection::{CodexConnection, read_message},
    contract::{CodexClientInfo, CodexInbound, CodexRequestId, CodexUserInput, TurnResponse},
    skills::Skills,
    tool,
    wire::{encode_error_response, encode_request, encode_response},
};
use crate::{
    agents::{
        AgentLaunchConfig, CommonTool, PeerMessage, SessionGoal, TokenUsage, ToolReviewState,
        WorkerActivity, WorkerActivityState, WorkerContext, WorkerEvent, WorkerInput,
        WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
        WorkerUsage,
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
        let mut command = self.command.clone();
        command.access_mode = launch.access_mode;
        command.app_proxy = launch.app_proxy.clone();
        let mut prepared = command.command(&launch.project)?;
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared()
            .issue_as_with_access(
                &launch.project,
                crate::modules::agents::core::CallerProfile {
                    backend: "codex-cli".into(),
                    provider: launch.provider.clone(),
                    model: launch.model.clone(),
                    effort: launch.effort.clone(),
                },
                None,
                launch.worker_id.clone(),
                launch.worker_name.clone(),
                launch.parent_worker_id.clone(),
                launch.access_mode,
            )?
            .with_slot(launch.slot.clone());
        configure_codex_app_server(&mut prepared, launch.access_mode);
        let mut child = prepared
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start Codex worker app-server: {error}"))?;
        child_stderr::capture(&mut child, "codex-worker")?;
        let ((mut reader, writer, queued, next_id, thread), skills) =
            match setup_connection(&mut child, &launch, launch.access_mode) {
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
            writer: Some(writer),
            incoming,
            thread_id: thread_id.clone(),
            model: launch.model,
            effort: launch.effort,
            collaboration_mode: None,
            collaboration_modes: HashMap::new(),
            command_state: commands::State::new(launch.access_mode),
            skills,
            project: launch.project.clone(),
            native_queue: false,
            next_id,
            current_turn: None,
            abort_starting_turn: false,
            output: String::new(),
            reasoning_started: false,
            compacting: false,
            manual_compaction: false,
            pending: HashMap::new(),
            pending_inputs: HashMap::new(),
            prompt_requests: HashMap::new(),
            client_submissions: HashMap::new(),
            native_inputs: HashMap::new(),
            native_input_order: VecDeque::new(),
            handoff: None,
            batch_deliveries: HashMap::new(),
            normal_start_clients: HashMap::new(),
            prompt_acks: VecDeque::new(),
            acknowledged_prompts: HashSet::new(),
            queued_inbound: VecDeque::new(),
            peer_messages: VecDeque::new(),
            events: VecDeque::from([WorkerEvent::SessionChanged { locator: thread_id }]),
            turn_error: None,
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
        load_main_metadata(&mut connection, project).map(|(metadata, _)| metadata)
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
    let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue_with_access(
        &launch.project,
        crate::modules::agents::core::CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
        command.access_mode,
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
    let setup = setup_main_connection(&mut child, launch, command.access_mode);
    let ((mut reader, writer, queued, next_id, thread), metadata, skills) = match setup {
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
    let mut session = CodexWorkerSession {
        caller_identity,
        child,
        writer: Some(writer),
        incoming,
        thread_id: thread_id.clone(),
        model: None,
        effort: None,
        collaboration_mode: None,
        collaboration_modes,
        command_state: commands::State::new(command.access_mode),
        skills,
        project: launch.project.clone(),
        native_queue: true,
        next_id,
        current_turn: None,
        abort_starting_turn: false,
        output: String::new(),
        reasoning_started: false,
        compacting: false,
        manual_compaction: false,
        pending: HashMap::new(),
        pending_inputs: HashMap::new(),
        prompt_requests: HashMap::new(),
        client_submissions: HashMap::new(),
        native_inputs: HashMap::new(),
        native_input_order: VecDeque::new(),
        handoff: None,
        batch_deliveries: HashMap::new(),
        normal_start_clients: HashMap::new(),
        prompt_acks: VecDeque::new(),
        acknowledged_prompts: HashSet::new(),
        queued_inbound: VecDeque::new(),
        peer_messages: VecDeque::new(),
        events: VecDeque::new(),
        turn_error: None,
    };
    let goal_request =
        session.request("thread/goal/get", json!({"threadId": thread_id.clone()}))?;
    session
        .pending
        .insert(goal_request, PendingRequest::LoadGoal);
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

fn setup_connection(
    child: &mut Child,
    launch: &WorkerLaunch,
    access_mode: crate::agents::HarnessAccessMode,
) -> Result<(CodexSetup, Skills), String> {
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
        WorkerContext::Fresh if launch.ephemeral => connection.start_ephemeral_thread(
            &cwd,
            launch.provider.as_deref(),
            launch.model.as_deref(),
            access_mode,
        )?,
        WorkerContext::Fresh => connection.start_thread(
            &cwd,
            launch.provider.as_deref(),
            launch.model.as_deref(),
            access_mode,
        )?,
        WorkerContext::Session { .. } if launch.ephemeral => {
            return Err("Codex cannot combine ephemeral inference with inherited context".into());
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
                access_mode,
            )?
        }
        WorkerContext::Resume { .. } if launch.ephemeral => {
            return Err("Codex cannot resume an ephemeral child worker".into());
        }
        WorkerContext::Resume { session_locator } => {
            connection.resume_thread(session_locator, access_mode)?
        }
    };
    let skills = Skills::load(&mut connection, &launch.project);
    let (reader, writer, queued, next_id) = connection.into_parts();
    Ok(((reader, writer, queued, next_id, thread), skills))
}

fn setup_main_connection(
    child: &mut Child,
    launch: &crate::agents::SessionLaunch,
    access_mode: crate::agents::HarnessAccessMode,
) -> Result<
    (
        CodexSetup,
        crate::modules::agents::adapter::main_session::MainSessionMetadata,
        Skills,
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
    let (mut metadata, skills) = load_main_metadata(&mut connection, &launch.project)?;
    let cwd = launch.project.to_string_lossy();
    let thread = match &launch.start {
        crate::agents::SessionStart::New => {
            connection.start_thread(&cwd, None, None, access_mode)?
        }
        crate::agents::SessionStart::Resume(_) => {
            let thread_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "Codex resume requires a thread id".to_owned())?;
            connection.resume_thread(&thread_id, access_mode)?
        }
        crate::agents::SessionStart::Fork(_) => {
            let thread_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "Codex fork requires a thread id".to_owned())?;
            connection.fork_thread(&thread_id, &cwd, None, None, access_mode)?
        }
    };
    metadata.session_name = thread.name.clone();
    let (reader, writer, queued, next_id) = connection.into_parts();
    Ok(((reader, writer, queued, next_id, thread), metadata, skills))
}

fn load_main_metadata(
    connection: &mut CodexConnection<BufReader<std::process::ChildStdout>, ChildStdin>,
    project: &std::path::Path,
) -> Result<
    (
        crate::modules::agents::adapter::main_session::MainSessionMetadata,
        Skills,
    ),
    String,
> {
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
            let model_efforts = supported_model_efforts(model);
            let efforts_known = model.get("supportedReasoningEfforts").is_some();
            for effort in &model_efforts {
                if !efforts.iter().any(|known| known == effort) {
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
                "efforts": efforts_known.then_some(model_efforts),
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
    let skills = Skills::load(connection, project);
    Ok((
        crate::modules::agents::adapter::main_session::MainSessionMetadata {
            models,
            efforts,
            commands: commands::catalog(&skills),
            modes,
            ..Default::default()
        },
        skills,
    ))
}

fn supported_model_efforts(model: &Value) -> Vec<String> {
    model
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|effort| {
            effort
                .as_str()
                .or_else(|| effort.get("reasoningEffort")?.as_str())
        })
        .map(str::to_owned)
        .collect()
}

enum PendingRequest {
    Command(commands::Request),
    LoadSkills,
    ObsoleteSkills,
    StartTurn,
    LoadGoal,
    ChildStatus {
        id: String,
        title: Option<String>,
    },
    ObsoleteChildStatus,
    Ignore,
    Control {
        operation: &'static str,
        client_id: Option<String>,
    },
    QueueDelete {
        client_id: String,
    },
    HandoffTurn {
        client_id: String,
        starts_turn: bool,
    },
}

#[derive(Clone)]
struct NativeInputDelivery {
    submission_id: Option<String>,
    mode: WorkerSendMode,
    message: String,
    images: Vec<crate::protocol::PromptImage>,
}

impl NativeInputDelivery {
    fn activity(self) -> WorkerActivity {
        match (self.submission_id, self.images.is_empty()) {
            (Some(submission_id), true) => WorkerActivity::SubmittedInputDelivered {
                submission_id,
                mode: self.mode,
                message: self.message,
            },
            (Some(submission_id), false) => WorkerActivity::SubmittedInputDeliveredWithImages {
                submission_id,
                mode: self.mode,
                message: self.message,
                images: self.images,
            },
            (None, true) => WorkerActivity::InputDelivered {
                mode: self.mode,
                message: self.message,
            },
            (None, false) => WorkerActivity::InputDeliveredWithImages {
                mode: self.mode,
                message: self.message,
                images: self.images,
            },
        }
    }
}

enum SteerReceipt {
    Pending,
    Accepted,
    RejectedByTurnRace,
}

enum NativeInputKind {
    Steer {
        receipt: SteerReceipt,
    },
    Queue {
        queue_id: Option<String>,
        claim_pending: bool,
        claimed: bool,
        claim_lost: bool,
    },
    Unknown,
}

struct PendingNativeInput {
    input: Vec<CodexUserInput>,
    delivery: NativeInputDelivery,
    kind: NativeInputKind,
    handoff: bool,
    cancel_on_delivery: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HandoffPhase {
    Interrupting,
    Claiming,
    Submitted,
}

struct Handoff {
    phase: HandoffPhase,
    cancelled: bool,
    wait_for_active_turn: bool,
    target_turn: Option<String>,
    batch_client_id: Option<String>,
}

struct CodexWorkerSession {
    caller_identity: crate::modules::agents::core::CallerIdentity,
    child: Child,
    writer: Option<ChildStdin>,
    incoming: mpsc::Receiver<Result<CodexInbound, String>>,
    thread_id: String,
    model: Option<String>,
    effort: Option<String>,
    collaboration_mode: Option<Value>,
    collaboration_modes: HashMap<String, Value>,
    command_state: commands::State,
    skills: Skills,
    project: std::path::PathBuf,
    native_queue: bool,
    next_id: i64,
    current_turn: Option<String>,
    abort_starting_turn: bool,
    output: String,
    reasoning_started: bool,
    compacting: bool,
    manual_compaction: bool,
    pending: HashMap<CodexRequestId, PendingRequest>,
    pending_inputs: HashMap<String, CodexRequestId>,
    prompt_requests: HashMap<CodexRequestId, String>,
    client_submissions: HashMap<String, String>,
    native_inputs: HashMap<String, PendingNativeInput>,
    native_input_order: VecDeque<String>,
    handoff: Option<Handoff>,
    batch_deliveries: HashMap<String, Vec<(NativeInputDelivery, bool)>>,
    normal_start_clients: HashMap<CodexRequestId, String>,
    prompt_acks: VecDeque<(String, Result<(), String>)>,
    acknowledged_prompts: HashSet<String>,
    queued_inbound: VecDeque<Result<CodexInbound, String>>,
    peer_messages: VecDeque<PeerMessage>,
    events: VecDeque<WorkerEvent>,
    turn_error: Option<String>,
}

impl WorkerSession for CodexWorkerSession {
    fn tracks_prompt_delivery(&self, _mode: WorkerSendMode) -> bool {
        true
    }

    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_with_images(message, mode, Vec::new())
    }

    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        if self.dispatch_command(&message, mode, &images, None)? {
            return Ok(());
        }
        self.send_prompt_input(message, mode, images, None)
    }

    fn submit_prompt(
        &mut self,
        id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        if self.dispatch_command(&message, mode, &images, Some(id.clone()))? {
            return Ok(false);
        }
        self.send_prompt_input(message, mode, images, Some(&id))?;
        self.prompt_requests
            .insert(CodexRequestId::Number(self.next_id), id);
        Ok(false)
    }

    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), String>)> {
        self.prompt_acks.pop_front()
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
        let encoded = encode_response(&request_id, result)?;
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| "Codex worker input is closed".to_owned())?;
        writer
            .write_all(&encoded)
            .and_then(|()| writer.flush())
            .map_err(|error| format!("answer Codex worker request: {error}"))
    }

    fn abort(&mut self) -> Result<(), String> {
        let handoff_phase = self.handoff.as_ref().map(|handoff| handoff.phase);
        if handoff_phase.is_some() {
            self.cancel_handoff()?;
            if handoff_phase != Some(HandoffPhase::Submitted) {
                return Ok(());
            }
        }
        let Some(turn_id) = self.current_turn.clone() else {
            if self.pending.values().any(|request| {
                matches!(
                    request,
                    PendingRequest::StartTurn
                        | PendingRequest::HandoffTurn {
                            starts_turn: true,
                            ..
                        }
                )
            }) {
                self.abort_starting_turn = true;
            }
            return Ok(());
        };
        self.interrupt_turn(&turn_id)
    }

    fn apply_steering(&mut self) -> Result<(), String> {
        if self.handoff.is_some() {
            return Ok(());
        }
        let mut has_pending = false;
        for input in self.native_inputs.values_mut() {
            if matches!(input.kind, NativeInputKind::Unknown) {
                continue;
            }
            input.handoff = true;
            has_pending = true;
        }
        if !has_pending {
            return Ok(());
        }
        self.handoff = Some(Handoff {
            phase: HandoffPhase::Interrupting,
            cancelled: false,
            wait_for_active_turn: false,
            target_turn: self.current_turn.clone(),
            batch_client_id: None,
        });
        if let Some(turn_id) = self.current_turn.clone() {
            self.interrupt_turn(&turn_id)
        } else if self
            .pending
            .values()
            .any(|request| matches!(request, PendingRequest::StartTurn))
        {
            self.abort_starting_turn = true;
            Ok(())
        } else {
            self.begin_handoff_claims()
        }
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
        self.wait_response(&id, "rename thread")
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
            self.peer_messages.push_back(message);
        }
        if let Some(mode) = WorkerSendMode::for_peer(self.activity())
            && !self.peer_messages.is_empty()
            && self.caller_identity.try_activate()
            && let Some(message) = self.peer_messages.pop_front()
        {
            return Some(match self.send_peer_message(&message, mode) {
                Ok(()) => WorkerEvent::Activity(WorkerActivity::PeerInputDelivered { message }),
                Err(error) => WorkerEvent::Failed(error),
            });
        }
        loop {
            let inbound = self
                .queued_inbound
                .pop_front()
                .or_else(|| self.incoming.try_recv().ok())?;
            // Correlate the actual RPC reply, never a write or turn notification.
            match &inbound {
                Ok(CodexInbound::Response { id, result }) => {
                    if let Some(prompt) = self.prompt_requests.get(id).cloned() {
                        let accepted = match self.pending.get(id) {
                            Some(PendingRequest::StartTurn) => {
                                serde_json::from_value::<TurnResponse>(result.clone())
                                    .map(|_| ())
                                    .map_err(|error| {
                                        format!("decode Codex turn acknowledgement: {error}")
                                    })
                            }
                            Some(PendingRequest::Control {
                                operation: "queue", ..
                            }) => queue_submission_id(result).map(|_| ()),
                            Some(PendingRequest::Control {
                                operation: "steer", ..
                            }) => result
                                .get("turnId")
                                .and_then(Value::as_str)
                                .map(|_| ())
                                .ok_or_else(|| {
                                    "decode Codex steer acknowledgement: missing turnId".into()
                                }),
                            _ => Ok(()),
                        };
                        if accepted.is_ok() {
                            self.prompt_requests.remove(id);
                            if let Some(PendingRequest::Control {
                                client_id: Some(client_id),
                                ..
                            }) = self.pending.get(id)
                                && let Some(input) = self.native_inputs.get_mut(client_id)
                                && let NativeInputKind::Steer { receipt } = &mut input.kind
                            {
                                *receipt = SteerReceipt::Accepted;
                            }
                            self.record_prompt_ack(prompt, Ok(()));
                        }
                    }
                }
                _ => {}
            }
            match inbound {
                Ok(CodexInbound::Response { id, result }) => match self.pending.remove(&id) {
                    Some(PendingRequest::Command(request)) => {
                        self.command_response(request, Ok(result));
                        if let Some(event) = self.events.pop_front() {
                            return Some(event);
                        }
                    }
                    Some(PendingRequest::LoadSkills) => {
                        match Skills::parse(result, &self.project) {
                            Ok(skills) => {
                                let commands = commands::catalog(&skills);
                                self.skills = skills;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::CommandsChanged { commands },
                                ));
                            }
                            Err(error) => {
                                zlog::warn!("Codex skills could not be refreshed: {error}");
                            }
                        }
                    }
                    Some(PendingRequest::StartTurn) => {
                        self.normal_start_clients.remove(&id);
                        let turn = match serde_json::from_value::<TurnResponse>(result) {
                            Ok(response) => response.turn,
                            Err(error) => {
                                let submission_id = self.prompt_requests.remove(&id)?;
                                return Some(WorkerEvent::PromptDeliveryUnknown {
                                    submission_id,
                                    error: format!("decode Codex worker turn: {error}"),
                                });
                            }
                        };
                        let started = self.begin_turn(&turn.id);
                        self.capture_handoff_target(&turn.id);
                        if let Err(error) = self.interrupt_started_turn_if_requested() {
                            return Some(WorkerEvent::Failed(error));
                        }
                        if started {
                            return Some(WorkerEvent::Started);
                        }
                    }
                    Some(PendingRequest::LoadGoal) => {
                        let Some(goal) = result.get("goal") else {
                            log_bad_codex_notification(
                                "thread/goal/get",
                                &result,
                                "goal response is missing goal",
                            );
                            continue;
                        };
                        match decode_codex_goal(goal) {
                            Ok(goal) => {
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::SessionGoalChanged(goal),
                                ));
                            }
                            Err(reason) => {
                                log_bad_codex_notification("thread/goal/get", &result, &reason);
                            }
                        }
                    }
                    Some(PendingRequest::ChildStatus { id, title }) => {
                        if let Some(is_running) = super::subagents::observe_thread(
                            &self.thread_id,
                            &id,
                            &result["thread"],
                        ) {
                            return Some(WorkerEvent::Activity(
                                WorkerActivity::ChildSessionsChanged {
                                    id,
                                    title,
                                    is_running,
                                    outcome: codex_child_thread_outcome(&result["thread"]),
                                },
                            ));
                        }
                    }
                    Some(PendingRequest::Control {
                        operation,
                        client_id,
                    }) => {
                        if let Some(client_id) = client_id.as_deref()
                            && let Err(error) = self.control_response(operation, client_id, &result)
                        {
                            if let Some(submission_id) = self.prompt_requests.remove(&id) {
                                if let Some(input) = self.native_inputs.get_mut(client_id) {
                                    input.kind = NativeInputKind::Unknown;
                                    input.handoff = false;
                                }
                                if let Err(handoff_error) = self.maybe_submit_handoff() {
                                    self.events.push_back(WorkerEvent::Failed(handoff_error));
                                }
                                return Some(WorkerEvent::PromptDeliveryUnknown {
                                    submission_id,
                                    error,
                                });
                            }
                            return Some(WorkerEvent::Failed(error));
                        }
                    }
                    Some(PendingRequest::QueueDelete { client_id }) => {
                        let deleted = result.get("deleted").and_then(Value::as_bool);
                        if let Err(error) = self.queue_delete_response(&client_id, deleted) {
                            return Some(WorkerEvent::Failed(error));
                        }
                    }
                    Some(PendingRequest::HandoffTurn {
                        client_id,
                        starts_turn,
                    }) => {
                        let started_turn = if starts_turn {
                            match serde_json::from_value::<TurnResponse>(result) {
                                Ok(response) => Some(response.turn),
                                Err(error) => {
                                    return self.unknown_handoff_delivery(
                                        &client_id,
                                        format!("decode Codex handoff turn: {error}"),
                                    );
                                }
                            }
                        } else if result.get("turnId").and_then(Value::as_str).is_some() {
                            None
                        } else {
                            return self.unknown_handoff_delivery(
                                &client_id,
                                "decode Codex handoff steer: missing turnId".into(),
                            );
                        };
                        let mut admitted = Vec::new();
                        if let Some(deliveries) = self.batch_deliveries.get_mut(&client_id) {
                            for (delivery, needs_ack) in deliveries {
                                if *needs_ack {
                                    if let Some(submission_id) = delivery.submission_id.clone() {
                                        admitted.push(submission_id);
                                    }
                                    *needs_ack = false;
                                }
                            }
                        }
                        for submission_id in admitted {
                            self.record_prompt_ack(submission_id, Ok(()));
                        }
                        if let Some(turn) = started_turn {
                            let started = self.begin_turn(&turn.id);
                            if let Err(error) = self.interrupt_started_turn_if_requested() {
                                return Some(WorkerEvent::Failed(error));
                            }
                            if started {
                                self.events.push_back(WorkerEvent::Started);
                            }
                        }
                    }
                    Some(
                        PendingRequest::Ignore
                        | PendingRequest::ObsoleteChildStatus
                        | PendingRequest::ObsoleteSkills,
                    )
                    | None => {}
                },
                Ok(CodexInbound::Error { id, error }) => {
                    let retry_handoff_steer = match self.pending.get(&id) {
                        Some(PendingRequest::Control {
                            operation: "steer",
                            client_id: Some(client_id),
                        }) => self.native_inputs.get(client_id).is_some_and(|input| {
                            steer_rejected_by_turn_race(&error)
                                && input.handoff
                                && self.handoff.as_ref().is_some_and(|handoff| {
                                    !handoff.cancelled
                                        && matches!(
                                            handoff.phase,
                                            HandoffPhase::Interrupting | HandoffPhase::Claiming
                                        )
                                })
                        }),
                        _ => false,
                    };
                    let rejected_prompt = self.prompt_requests.remove(&id);
                    if let Some(prompt) = rejected_prompt.as_ref() {
                        if !retry_handoff_steer {
                            self.record_prompt_ack(prompt.clone(), Err(error.message.clone()));
                        }
                    }
                    match self.pending.remove(&id) {
                        Some(PendingRequest::Command(request)) => {
                            self.command_response(request, Err(error.message));
                            return self.events.pop_front();
                        }
                        Some(PendingRequest::LoadSkills) => {
                            zlog::warn!("Codex skills could not be refreshed: {}", error.message);
                            continue;
                        }
                        Some(
                            PendingRequest::ObsoleteChildStatus | PendingRequest::ObsoleteSkills,
                        ) => continue,
                        Some(PendingRequest::ChildStatus { .. }) => {
                            zlog::warn!("Codex child status could not be read: {}", error.message);
                            continue;
                        }
                        Some(PendingRequest::LoadGoal) => {
                            zlog::warn!(
                                "Codex thread goal could not be read: {}: {}",
                                error.code,
                                error.message
                            );
                            continue;
                        }
                        Some(PendingRequest::StartTurn) => {
                            if let Some(client_id) = self.normal_start_clients.remove(&id) {
                                self.batch_deliveries.remove(&client_id);
                            }
                            self.abort_starting_turn = false;
                            self.caller_identity.set_activity(WorkerActivityState::Idle);
                        }
                        Some(PendingRequest::Control {
                            operation,
                            client_id,
                        }) => {
                            if retry_handoff_steer {
                                if let Some(client_id) = client_id.as_deref()
                                    && let Some(input) = self.native_inputs.get_mut(client_id)
                                    && let NativeInputKind::Steer { receipt } = &mut input.kind
                                {
                                    *receipt = SteerReceipt::RejectedByTurnRace;
                                }
                                if let Err(submit_error) = self.maybe_submit_handoff() {
                                    return Some(WorkerEvent::Failed(submit_error));
                                }
                                continue;
                            }
                            if let Some(client_id) = client_id {
                                self.client_submissions.remove(&client_id);
                                self.native_inputs.remove(&client_id);
                                self.native_input_order
                                    .retain(|queued| queued != &client_id);
                            }
                            zlog::warn!(
                                "Codex {operation} request was rejected: {}",
                                error.message
                            );
                            if rejected_prompt.is_some() {
                                continue;
                            }
                            return Some(WorkerEvent::RequestFailed {
                                operation: format!("Codex {operation}"),
                                error: error.message,
                            });
                        }
                        Some(PendingRequest::QueueDelete { client_id }) => {
                            if let Some(input) = self.native_inputs.get_mut(&client_id) {
                                input.handoff = false;
                                if let NativeInputKind::Queue { claim_pending, .. } =
                                    &mut input.kind
                                {
                                    *claim_pending = false;
                                }
                            }
                            if let Err(submit_error) = self.maybe_submit_handoff() {
                                return Some(WorkerEvent::Failed(submit_error));
                            }
                            zlog::warn!(
                                "Codex queue delete request was rejected: {}",
                                error.message
                            );
                            continue;
                        }
                        Some(PendingRequest::HandoffTurn { client_id, .. }) => {
                            if let Some(deliveries) = self.batch_deliveries.remove(&client_id) {
                                for (delivery, needs_ack) in deliveries {
                                    if needs_ack && let Some(submission_id) = delivery.submission_id
                                    {
                                        self.record_prompt_ack(
                                            submission_id,
                                            Err(error.message.clone()),
                                        );
                                    }
                                }
                            }
                            if self.handoff.as_ref().is_some_and(|handoff| {
                                handoff.batch_client_id.as_deref() == Some(client_id.as_str())
                            }) {
                                self.handoff = None;
                            }
                            return Some(WorkerEvent::RequestFailed {
                                operation: "Codex steering handoff".into(),
                                error: error.message,
                            });
                        }
                        Some(PendingRequest::Ignore) | None => {}
                    }
                    self.manual_compaction = false;
                    self.compacting = false;
                    return Some(WorkerEvent::Failed(format!(
                        "Codex app-server error {}: {}",
                        error.code, error.message
                    )));
                }
                Ok(CodexInbound::Notification { method, params }) => {
                    if method == "skills/changed" {
                        match self.request("skills/list", Skills::params(&self.project, true)) {
                            Ok(id) => {
                                for pending in self.pending.values_mut() {
                                    if matches!(pending, PendingRequest::LoadSkills) {
                                        *pending = PendingRequest::ObsoleteSkills;
                                    }
                                }
                                self.pending.insert(id, PendingRequest::LoadSkills);
                            }
                            Err(error) => {
                                zlog::warn!("Codex skills could not be refreshed: {error}");
                            }
                        }
                        continue;
                    }
                    if let Some(activity) = codex_telemetry(&method, &params) {
                        return Some(WorkerEvent::Activity(activity));
                    }
                    if matches!(
                        method.as_str(),
                        "account/rateLimits/updated" | "mcpServer/startupStatus/updated"
                    ) {
                        log_bad_codex_notification(
                            &method,
                            &params,
                            "telemetry update is missing required fields",
                        );
                        continue;
                    }
                    if !codex_notification_is_for_thread(&method, &params, &self.thread_id) {
                        continue;
                    }
                    match method.as_str() {
                        "turn/started" => {
                            if let Some(turn_id) = params["turn"]["id"].as_str() {
                                let started = self.begin_turn(turn_id);
                                self.capture_handoff_target(turn_id);
                                if let Err(error) = self.maybe_submit_handoff() {
                                    return Some(WorkerEvent::Failed(error));
                                }
                                if let Err(error) = self.interrupt_started_turn_if_requested() {
                                    return Some(WorkerEvent::Failed(error));
                                }
                                if started {
                                    return Some(WorkerEvent::Started);
                                }
                            } else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "turn start is missing turn id",
                                );
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
                            log_bad_codex_notification(
                                &method,
                                &params,
                                "agent delta is missing delta",
                            );
                        }
                        "item/plan/delta" => {
                            if let Some(delta) = params["delta"].as_str() {
                                self.reasoning_started = true;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::ThinkingDelta {
                                        content_index: 0,
                                        delta: delta.to_owned(),
                                    },
                                ));
                            }
                            log_bad_codex_notification(
                                &method,
                                &params,
                                "plan delta is missing delta",
                            );
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
                            log_bad_codex_notification(
                                &method,
                                &params,
                                "reasoning delta is missing delta",
                            );
                        }
                        "item/started" => {
                            if let Some(activity) = self.input_delivery(&params["item"]) {
                                return Some(WorkerEvent::Activity(activity));
                            }
                            let item_type = params.pointer("/item/type").and_then(Value::as_str);
                            // Native child activity is an instantaneous event, projected once
                            // from item/completed along with a catalog refresh.
                            if item_type == Some("subAgentActivity") {
                                continue;
                            }
                            if item_type == Some("reasoning") {
                                self.reasoning_started = true;
                            }
                            if item_type == Some("contextCompaction") {
                                self.compacting = true;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::CompactionStarted,
                                ));
                            }
                            if let Some(event) = codex_tool_start(&params) {
                                return Some(WorkerEvent::Activity(event));
                            }
                            if !codex_passive_item(&params["item"]) {
                                log_bad_codex_notification(&method, &params, "unmapped item start");
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
                            log_bad_codex_notification(
                                &method,
                                &params,
                                "command output delta is missing itemId or delta",
                            );
                        }
                        "item/completed" => {
                            let item_type = params.pointer("/item/type").and_then(Value::as_str);
                            if item_type == Some("exitedReviewMode") {
                                if let Some(review) = params["item"]["review"].as_str() {
                                    self.output = review.to_owned();
                                }
                            }
                            if let Some(output) = codex_agent_message_text(&params["item"]) {
                                self.output = output;
                            }
                            if item_type == Some("contextCompaction") {
                                self.compacting = false;
                                return Some(WorkerEvent::Activity(
                                    WorkerActivity::CompactionFinished {
                                        aborted: false,
                                        error: None,
                                    },
                                ));
                            }
                            if matches!(item_type, Some("webSearch" | "subAgentActivity"))
                                && let Some(started) = codex_tool_start(&params)
                                && let Some(finished) = codex_tool_end(&params)
                            {
                                let event = if item_type == Some("subAgentActivity")
                                    && let Some(activity) =
                                        self.observe_child_activity(&params["item"])
                                {
                                    self.events.push_back(WorkerEvent::Activity(started));
                                    activity
                                } else {
                                    started
                                };
                                self.events.push_back(WorkerEvent::Activity(finished));
                                return Some(WorkerEvent::Activity(event));
                            }
                            if let Some(finished) = codex_tool_end(&params) {
                                if let Some(changed) = codex_tool_metadata_changed(&params) {
                                    self.events.push_back(WorkerEvent::Activity(finished));
                                    return Some(WorkerEvent::Activity(changed));
                                }
                                return Some(WorkerEvent::Activity(finished));
                            }
                            if !codex_passive_item(&params["item"]) {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "unmapped item completion",
                                );
                            }
                        }
                        "item/autoApprovalReview/started" => {
                            let Some((started, review)) = codex_tool_review_started(&params) else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "approval review start is not a tool review",
                                );
                                continue;
                            };
                            self.events.push_back(WorkerEvent::Activity(review));
                            return Some(WorkerEvent::Activity(started));
                        }
                        "item/autoApprovalReview/completed" => {
                            let Some((review, finished)) = codex_tool_review_completed(&params)
                            else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "approval review completion is missing targetItemId",
                                );
                                continue;
                            };
                            if let Some(finished) = finished {
                                self.events.push_back(WorkerEvent::Activity(finished));
                            }
                            return Some(WorkerEvent::Activity(review));
                        }
                        "guardianWarning" => {}
                        "thread/tokenUsage/updated" => {
                            let Some(usage) = params.get("tokenUsage") else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "token update is missing tokenUsage",
                                );
                                continue;
                            };
                            let Some(total) = usage.get("total") else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "token update is missing total",
                                );
                                continue;
                            };
                            let session = codex_usage(total);
                            let turn = usage.get("last").map(codex_usage).unwrap_or(session);
                            let reported_usage = WorkerUsage {
                                turn,
                                session,
                                context_window: usage
                                    .get("modelContextWindow")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0),
                            };
                            self.command_state.usage = Some(reported_usage);
                            return Some(WorkerEvent::Activity(WorkerActivity::Usage(
                                reported_usage,
                            )));
                        }
                        "thread/goal/updated" => {
                            let Some(goal) = params.get("goal") else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "goal update is missing goal",
                                );
                                continue;
                            };
                            match decode_codex_goal(goal) {
                                Ok(goal) => {
                                    return Some(WorkerEvent::Activity(
                                        WorkerActivity::SessionGoalChanged(goal),
                                    ));
                                }
                                Err(reason) => {
                                    log_bad_codex_notification(&method, &params, &reason);
                                }
                            }
                        }
                        "thread/goal/cleared" => {
                            return Some(WorkerEvent::Activity(
                                WorkerActivity::SessionGoalChanged(None),
                            ));
                        }
                        "turn/completed" => {
                            let Some(completed_turn) =
                                params["turn"]["id"].as_str().map(str::to_owned)
                            else {
                                log_bad_codex_notification(
                                    &method,
                                    &params,
                                    "turn completion is missing turn id",
                                );
                                continue;
                            };
                            if self.current_turn.as_deref() != Some(completed_turn.as_str()) {
                                continue;
                            }
                            self.current_turn = None;
                            self.caller_identity.set_activity(WorkerActivityState::Idle);
                            if self.handoff.as_ref().is_some_and(|handoff| {
                                handoff.phase == HandoffPhase::Interrupting
                                    && handoff.target_turn.as_deref()
                                        == Some(completed_turn.as_str())
                            }) {
                                if params["turn"]["status"].as_str() == Some("interrupted") {
                                    if let Err(error) = self.begin_handoff_claims() {
                                        return Some(WorkerEvent::Failed(error));
                                    }
                                    self.discard_cancelled_steers();
                                } else if let Some(handoff) = self.handoff.as_mut() {
                                    handoff.cancelled = true;
                                }
                            }
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
                                return Some(WorkerEvent::Failed(codex_turn_failure(
                                    self.turn_error.take(),
                                )));
                            }
                            return Some(WorkerEvent::Settled {
                                output: self.output.clone(),
                            });
                        }
                        "item/reasoning/summaryPartAdded" => {
                            self.reasoning_started = true;
                        }
                        "error" => {
                            if let Some(message) = codex_error_message(&params) {
                                self.turn_error = Some(message);
                            }
                        }
                        "thread/settings/updated" => {
                            self.observe_command_settings(&params["threadSettings"])
                        }
                        "thread/name/updated" => {
                            if let Some(name) = params.get("threadName").and_then(Value::as_str) {
                                return Some(WorkerEvent::Activity(WorkerActivity::TitleChanged(
                                    name.to_owned(),
                                )));
                            }
                        }
                        "thread/status/changed"
                        | "turn/diff/updated"
                        | "turn/plan/updated"
                        | "serverRequest/resolved"
                        | "item/commandExecution/terminalInteraction"
                        | "item/fileChange/outputDelta" => {}
                        _ => log_bad_codex_notification(
                            &method,
                            &params,
                            "unmapped same-thread notification",
                        ),
                    }
                }
                Ok(CodexInbound::ServerRequest { id, method, params }) => {
                    if !is_codex_approval_request(&method) {
                        zlog::warn!(
                            "Unsupported Codex server request was not mapped: id={id:?} method={method} params={params}"
                        );
                        let message = format!("unsupported Codex server request: {method}");
                        let rejected =
                            encode_error_response(&id, -32601, &message).and_then(|encoded| {
                                let writer = self
                                    .writer
                                    .as_mut()
                                    .ok_or_else(|| "Codex worker input is closed".to_owned())?;
                                writer
                                    .write_all(&encoded)
                                    .and_then(|()| writer.flush())
                                    .map_err(|error| error.to_string())
                            });
                        if let Err(error) = rejected {
                            return Some(WorkerEvent::Failed(format!(
                                "reject unsupported Codex server request: {error}"
                            )));
                        }
                        continue;
                    }
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
        super::subagents::forget_parent(&self.thread_id);
        self.writer.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if self
                .child
                .try_wait()
                .map_err(|error| format!("check Codex worker: {error}"))?
                .is_some()
            {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.child
            .kill()
            .map_err(|error| format!("terminate Codex worker: {error}"))?;
        self.child
            .wait()
            .map_err(|error| format!("reap Codex worker: {error}"))?;
        Ok(())
    }
}

impl CodexWorkerSession {
    fn record_prompt_ack(&mut self, id: String, result: Result<(), String>) {
        if self.acknowledged_prompts.insert(id.clone()) {
            self.prompt_acks.push_back((id, result));
        }
    }

    fn unknown_handoff_delivery(&mut self, client_id: &str, error: String) -> Option<WorkerEvent> {
        let mut unknown = Vec::new();
        if let Some(deliveries) = self.batch_deliveries.get_mut(client_id) {
            for (delivery, needs_ack) in deliveries {
                if *needs_ack {
                    if let Some(submission_id) = delivery.submission_id.clone() {
                        unknown.push(WorkerEvent::PromptDeliveryUnknown {
                            submission_id,
                            error: error.clone(),
                        });
                    }
                    *needs_ack = false;
                }
            }
        }
        if self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.batch_client_id.as_deref() == Some(client_id))
        {
            self.handoff = None;
        }
        let mut unknown = unknown.into_iter();
        let first = unknown.next();
        self.events.extend(unknown);
        first
    }

    fn input_delivery(&mut self, item: &Value) -> Option<WorkerActivity> {
        if item.get("type").and_then(Value::as_str) == Some("userMessage")
            && let Some(client_id) = item.get("clientId").and_then(Value::as_str)
        {
            if let Some(deliveries) = self.batch_deliveries.remove(client_id) {
                let mut activities = deliveries
                    .into_iter()
                    .map(|(delivery, _)| delivery.activity());
                let first = activities.next();
                self.events.extend(activities.map(WorkerEvent::Activity));
                if self
                    .handoff
                    .as_ref()
                    .is_some_and(|handoff| handoff.batch_client_id.as_deref() == Some(client_id))
                {
                    self.handoff = None;
                }
                return first;
            }
            if let Some(input) = self.native_inputs.remove(client_id) {
                self.native_input_order.retain(|queued| queued != client_id);
                self.client_submissions.remove(client_id);
                if input.cancel_on_delivery
                    && let Some(turn_id) = self.current_turn.clone()
                    && let Err(error) = self.interrupt_turn(&turn_id)
                {
                    self.events.push_back(WorkerEvent::Failed(error));
                }
                let activity = input.delivery.activity();
                self.finish_cancelled_handoff();
                if let Err(error) = self.maybe_submit_handoff() {
                    self.events.push_back(WorkerEvent::Failed(error));
                }
                return Some(activity);
            }
        }
        let activity = codex_input_delivery(item)?;
        let submission_id = item
            .get("clientId")
            .and_then(Value::as_str)
            .and_then(|client_id| self.client_submissions.remove(client_id));
        match (submission_id, activity) {
            (Some(submission_id), WorkerActivity::InputDelivered { mode, message }) => {
                Some(WorkerActivity::SubmittedInputDelivered {
                    submission_id,
                    mode,
                    message,
                })
            }
            (
                Some(submission_id),
                WorkerActivity::InputDeliveredWithImages {
                    mode,
                    message,
                    images,
                },
            ) => Some(WorkerActivity::SubmittedInputDeliveredWithImages {
                submission_id,
                mode,
                message,
                images,
            }),
            (_, activity) => Some(activity),
        }
    }

    fn send_prompt_input(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
        submission_id: Option<&str>,
    ) -> Result<(), String> {
        let delivery = NativeInputDelivery {
            submission_id: submission_id.map(str::to_owned),
            mode,
            message: message.clone(),
            images: images.clone(),
        };
        let mut input = self.skills.input(message);
        let inline_images = images
            .into_iter()
            .map(crate::protocol::PromptImage::into_inline)
            .collect::<Result<Vec<_>, _>>()?;
        input.extend(
            inline_images
                .into_iter()
                .map(|image| CodexUserInput::Image {
                    url: format!("data:{};base64,{}", image.mime_type, image.data),
                }),
        );
        self.send_input(input, mode, submission_id, delivery)
    }

    fn interrupt_turn(&mut self, turn_id: &str) -> Result<(), String> {
        let id = self.request(
            "turn/interrupt",
            json!({"threadId": self.thread_id, "turnId": turn_id}),
        )?;
        self.pending.insert(
            id,
            PendingRequest::Control {
                operation: "interrupt",
                client_id: None,
            },
        );
        Ok(())
    }

    fn capture_handoff_target(&mut self, turn_id: &str) {
        if let Some(handoff) = self.handoff.as_mut()
            && handoff.phase == HandoffPhase::Interrupting
            && handoff.target_turn.is_none()
        {
            handoff.target_turn = Some(turn_id.to_owned());
        }
    }

    fn interrupt_started_turn_if_requested(&mut self) -> Result<(), String> {
        if !self.abort_starting_turn {
            return Ok(());
        }
        let Some(turn_id) = self.current_turn.clone() else {
            return Ok(());
        };
        self.interrupt_turn(&turn_id)?;
        self.abort_starting_turn = false;
        Ok(())
    }

    fn control_response(
        &mut self,
        operation: &'static str,
        client_id: &str,
        result: &Value,
    ) -> Result<(), String> {
        if operation == "steer" {
            if result.get("turnId").and_then(Value::as_str).is_none() {
                return Err("decode Codex steer acknowledgement: missing turnId".into());
            }
            return self.maybe_submit_handoff();
        }
        if operation != "queue" {
            return Ok(());
        }
        let queue_id = queue_submission_id(result)?;
        let should_delete = if let Some(input) = self.native_inputs.get_mut(client_id) {
            let NativeInputKind::Queue {
                queue_id: stored_id,
                claim_pending,
                ..
            } = &mut input.kind
            else {
                return Ok(());
            };
            *stored_id = Some(queue_id.clone());
            let should_delete = input.handoff
                && self.handoff.as_ref().is_some_and(|handoff| {
                    handoff.cancelled || handoff.phase == HandoffPhase::Claiming
                });
            if should_delete {
                *claim_pending = true;
            }
            should_delete
        } else {
            false
        };
        if should_delete {
            self.delete_queued_input(client_id, &queue_id)?;
        }
        Ok(())
    }

    fn begin_handoff_claims(&mut self) -> Result<(), String> {
        let Some(handoff) = self.handoff.as_mut() else {
            return Ok(());
        };
        handoff.phase = HandoffPhase::Claiming;
        let mut deletes = Vec::new();
        for client_id in &self.native_input_order {
            let Some(input) = self.native_inputs.get_mut(client_id) else {
                continue;
            };
            if !input.handoff {
                continue;
            }
            if let NativeInputKind::Queue {
                queue_id: Some(queue_id),
                claim_pending,
                claimed,
                ..
            } = &mut input.kind
                && !*claim_pending
                && !*claimed
            {
                *claim_pending = true;
                deletes.push((client_id.clone(), queue_id.clone()));
            }
        }
        for (client_id, queue_id) in deletes {
            self.delete_queued_input(&client_id, &queue_id)?;
        }
        self.maybe_submit_handoff()
    }

    fn delete_queued_input(&mut self, client_id: &str, queue_id: &str) -> Result<(), String> {
        let id = self.request(
            "thread/queue/delete",
            json!({"threadId": self.thread_id, "queuedSubmissionId": queue_id}),
        )?;
        self.pending.insert(
            id,
            PendingRequest::QueueDelete {
                client_id: client_id.to_owned(),
            },
        );
        Ok(())
    }

    fn queue_delete_response(
        &mut self,
        client_id: &str,
        deleted: Option<bool>,
    ) -> Result<(), String> {
        let Some(input) = self.native_inputs.get_mut(client_id) else {
            return Ok(());
        };
        let NativeInputKind::Queue {
            claim_pending,
            claimed,
            claim_lost,
            ..
        } = &mut input.kind
        else {
            return Ok(());
        };
        *claim_pending = false;
        let cancelled = self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.cancelled);
        match deleted {
            Some(true) => *claimed = true,
            Some(false) => {
                *claim_lost = true;
                if let Some(handoff) = self.handoff.as_mut() {
                    handoff.wait_for_active_turn = true;
                    input.cancel_on_delivery = handoff.cancelled;
                }
            }
            None => {
                input.handoff = false;
                return Err("decode Codex queue deletion: missing deleted flag".into());
            }
        }
        if deleted == Some(true) && cancelled {
            self.native_inputs.remove(client_id);
            self.native_input_order.retain(|queued| queued != client_id);
            self.client_submissions.remove(client_id);
            self.finish_cancelled_handoff();
        }
        self.maybe_submit_handoff()
    }

    fn maybe_submit_handoff(&mut self) -> Result<(), String> {
        let Some(handoff) = self.handoff.as_ref() else {
            return Ok(());
        };
        if handoff.cancelled || handoff.phase != HandoffPhase::Claiming {
            return Ok(());
        }
        if handoff.wait_for_active_turn && self.current_turn.is_none() {
            return Ok(());
        }
        let mut selected = Vec::new();
        for client_id in &self.native_input_order {
            let Some(input) = self.native_inputs.get(client_id) else {
                continue;
            };
            if !input.handoff {
                continue;
            }
            match &input.kind {
                NativeInputKind::Steer {
                    receipt: SteerReceipt::Pending,
                } => return Ok(()),
                NativeInputKind::Steer {
                    receipt: SteerReceipt::Accepted | SteerReceipt::RejectedByTurnRace,
                } => selected.push(client_id.clone()),
                NativeInputKind::Unknown => {}
                NativeInputKind::Queue { queue_id: None, .. }
                | NativeInputKind::Queue {
                    claim_pending: true,
                    ..
                } => return Ok(()),
                NativeInputKind::Queue { claimed: true, .. } => selected.push(client_id.clone()),
                NativeInputKind::Queue { claimed: false, .. } => {}
            }
        }
        if selected.is_empty() {
            self.handoff = None;
            return Ok(());
        }

        let batch_client_id = format!(
            "{HANDOFF_CLIENT_ID_PREFIX}{}",
            self.next_id.saturating_add(1)
        );
        let mut batch_input = Vec::new();
        let mut deliveries = Vec::new();
        for client_id in &selected {
            let input = self
                .native_inputs
                .get(client_id)
                .expect("selected native input must still exist");
            if !batch_input.is_empty() {
                batch_input.push(CodexUserInput::text("\n\n"));
            }
            batch_input.extend(input.input.clone());
            let needs_ack = input
                .delivery
                .submission_id
                .as_ref()
                .is_some_and(|id| !self.acknowledged_prompts.contains(id));
            deliveries.push((input.delivery.clone(), needs_ack));
        }
        let active_turn = self.current_turn.clone();
        let (method, params, starts_turn) = if let Some(turn_id) = active_turn {
            (
                "turn/steer",
                json!({
                    "threadId": self.thread_id,
                    "expectedTurnId": turn_id,
                    "clientUserMessageId": batch_client_id,
                    "input": batch_input,
                }),
                false,
            )
        } else {
            (
                "turn/start",
                json!({
                    "threadId": self.thread_id,
                    "clientUserMessageId": batch_client_id,
                    "input": batch_input,
                    "model": self.model,
                    "effort": self.effort,
                    "collaborationMode": self.collaboration_mode,
                }),
                true,
            )
        };
        let id = self.submission_request(method, params)?;
        for client_id in selected {
            self.native_inputs.remove(&client_id);
            self.client_submissions.remove(&client_id);
            self.native_input_order
                .retain(|queued| queued != &client_id);
        }
        self.batch_deliveries
            .insert(batch_client_id.clone(), deliveries);
        if let Some(handoff) = self.handoff.as_mut() {
            handoff.phase = HandoffPhase::Submitted;
            handoff.batch_client_id = Some(batch_client_id.clone());
        }
        self.pending.insert(
            id,
            PendingRequest::HandoffTurn {
                client_id: batch_client_id,
                starts_turn,
            },
        );
        if starts_turn {
            self.caller_identity
                .set_activity(WorkerActivityState::Starting);
        }
        Ok(())
    }

    fn cancel_handoff(&mut self) -> Result<(), String> {
        let Some(handoff) = self.handoff.as_mut() else {
            return Ok(());
        };
        handoff.cancelled = true;
        let mut deletes = Vec::new();
        for (client_id, input) in &mut self.native_inputs {
            if !input.handoff {
                continue;
            }
            if let NativeInputKind::Queue {
                queue_id: Some(queue_id),
                claim_pending,
                claimed,
                claim_lost,
            } = &mut input.kind
            {
                if *claim_lost {
                    input.cancel_on_delivery = true;
                } else if !*claim_pending && !*claimed {
                    *claim_pending = true;
                    deletes.push((client_id.clone(), queue_id.clone()));
                }
            }
        }
        for (client_id, queue_id) in deletes {
            self.delete_queued_input(&client_id, &queue_id)?;
        }
        Ok(())
    }

    fn discard_cancelled_steers(&mut self) {
        if !self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.cancelled)
        {
            return;
        }
        let discarded = self
            .native_inputs
            .iter()
            .filter_map(|(client_id, input)| {
                (input.handoff && matches!(input.kind, NativeInputKind::Steer { .. }))
                    .then(|| client_id.clone())
            })
            .collect::<Vec<_>>();
        for client_id in discarded {
            self.native_inputs.remove(&client_id);
            self.native_input_order
                .retain(|queued| queued != &client_id);
        }
        self.finish_cancelled_handoff();
    }

    fn finish_cancelled_handoff(&mut self) {
        let Some(handoff) = self.handoff.as_ref().filter(|handoff| handoff.cancelled) else {
            return;
        };
        let current_batch = handoff.batch_client_id.clone();
        let has_originals = self.native_inputs.values().any(|input| input.handoff);
        let has_current_batch = current_batch
            .as_ref()
            .is_some_and(|client_id| self.batch_deliveries.contains_key(client_id));
        if !has_originals && !has_current_batch {
            self.handoff = None;
        }
    }

    fn wait_response(
        &mut self,
        request_id: &CodexRequestId,
        operation: &str,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            match self
                .incoming
                .recv_timeout(std::time::Duration::from_millis(50))
            {
                Ok(Ok(CodexInbound::Response { id, .. })) if &id == request_id => return Ok(()),
                Ok(Ok(CodexInbound::Error { id, error })) if &id == request_id => {
                    return Err(format!(
                        "Codex could not {operation}: {} ({})",
                        error.message, error.code
                    ));
                }
                Ok(inbound) => self.queued_inbound.push_back(inbound),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!("Codex stopped while attempting to {operation}"));
                }
            }
        }
        Err(format!("Codex did not {operation} within 15 seconds"))
    }

    fn begin_turn(&mut self, turn_id: &str) -> bool {
        let is_new = self.current_turn.as_deref() != Some(turn_id);
        self.current_turn = Some(turn_id.to_owned());
        self.caller_identity
            .set_activity(WorkerActivityState::Working);
        if is_new {
            self.output.clear();
            self.reasoning_started = false;
            self.turn_error = None;
            if !self.manual_compaction {
                self.reasoning_started = true;
                self.events
                    .push_back(WorkerEvent::Activity(WorkerActivity::ThinkingStarted {
                        content_index: 0,
                    }));
            }
        }
        is_new
    }

    fn activity(&self) -> WorkerActivityState {
        if self.current_turn.is_some() {
            WorkerActivityState::Working
        } else if self.pending.values().any(|request| {
            matches!(
                request,
                PendingRequest::StartTurn | PendingRequest::Command(_)
            )
        }) {
            WorkerActivityState::Starting
        } else {
            WorkerActivityState::Idle
        }
    }

    fn send_input(
        &mut self,
        input: Vec<CodexUserInput>,
        mode: WorkerSendMode,
        submission_id: Option<&str>,
        delivery: NativeInputDelivery,
    ) -> Result<(), String> {
        if mode == WorkerSendMode::Steer {
            let turn_id = self
                .current_turn
                .as_deref()
                .ok_or_else(|| "Codex worker has not reported its active turn".to_owned())?;
            let client_id = format!("{STEER_CLIENT_ID_PREFIX}{}", self.next_id.saturating_add(1));
            let id = self.submission_request(
                "turn/steer",
                json!({
                    "threadId": self.thread_id,
                    "expectedTurnId": turn_id,
                    "clientUserMessageId": client_id,
                    "input": input,
                }),
            )?;
            if let Some(submission_id) = submission_id {
                self.client_submissions
                    .insert(client_id.clone(), submission_id.into());
            }
            self.pending.insert(
                id,
                PendingRequest::Control {
                    operation: "steer",
                    client_id: Some(client_id.clone()),
                },
            );
            self.native_input_order.push_back(client_id.clone());
            self.native_inputs.insert(
                client_id,
                PendingNativeInput {
                    input,
                    delivery,
                    kind: NativeInputKind::Steer {
                        receipt: SteerReceipt::Pending,
                    },
                    handoff: false,
                    cancel_on_delivery: false,
                },
            );
            return Ok(());
        }
        if mode == WorkerSendMode::Queue && self.native_queue {
            let client_id = format!("{QUEUE_CLIENT_ID_PREFIX}{}", self.next_id.saturating_add(1));
            let id = self.submission_request(
                "thread/queue/add",
                json!({
                    "threadId": self.thread_id,
                    "clientUserMessageId": client_id,
                    "input": input,
                }),
            )?;
            if let Some(submission_id) = submission_id {
                self.client_submissions
                    .insert(client_id.clone(), submission_id.into());
            }
            self.pending.insert(
                id,
                PendingRequest::Control {
                    operation: "queue",
                    client_id: Some(client_id.clone()),
                },
            );
            self.native_input_order.push_back(client_id.clone());
            self.native_inputs.insert(
                client_id,
                PendingNativeInput {
                    input,
                    delivery,
                    kind: NativeInputKind::Queue {
                        queue_id: None,
                        claim_pending: false,
                        claimed: false,
                        claim_lost: false,
                    },
                    handoff: false,
                    cancel_on_delivery: false,
                },
            );
            return Ok(());
        }
        self.output.clear();
        self.reasoning_started = false;
        let client_id = format!(
            "{NORMAL_CLIENT_ID_PREFIX}{}",
            self.next_id.saturating_add(1)
        );
        let id = self.submission_request(
            "turn/start",
            json!({
                "threadId": self.thread_id,
                "clientUserMessageId": client_id,
                "input": input,
                "model": self.model,
                "effort": self.effort,
                "collaborationMode": self.collaboration_mode,
            }),
        )?;
        if submission_id.is_some() {
            self.batch_deliveries
                .insert(client_id.clone(), vec![(delivery, false)]);
            self.normal_start_clients.insert(id.clone(), client_id);
        }
        self.pending.insert(id, PendingRequest::StartTurn);
        self.caller_identity
            .set_activity(WorkerActivityState::Starting);
        Ok(())
    }

    fn observe_child_activity(&mut self, item: &Value) -> Option<WorkerActivity> {
        let child = item["agentThreadId"].as_str()?;
        let title = item["agentPath"].as_str().map(str::to_owned);
        // A later lifecycle event supersedes any status read still in flight.
        for pending in self.pending.values_mut() {
            if matches!(pending, PendingRequest::ChildStatus { id, .. } if id == child) {
                *pending = PendingRequest::ObsoleteChildStatus;
            }
        }
        if let Some(is_running) = super::subagents::observe(&self.thread_id, item) {
            return Some(WorkerActivity::ChildSessionsChanged {
                id: child.to_owned(),
                title,
                is_running,
                outcome: codex_child_event_outcome(item),
            });
        }
        if item["kind"].as_str() != Some("interacted") {
            return None;
        }
        // Both message delivery and follow-up turns emit interacted. Read the
        // child's actual turn instead of assigning a lifecycle to that event.
        match self.request(
            "thread/read",
            json!({"threadId": child, "includeTurns": true}),
        ) {
            Ok(request) => {
                self.pending.insert(
                    request,
                    PendingRequest::ChildStatus {
                        id: child.to_owned(),
                        title,
                    },
                );
            }
            Err(error) => {
                zlog::warn!("Codex child status could not be requested: {error}");
            }
        }
        None
    }

    fn request(&mut self, method: &str, params: Value) -> Result<CodexRequestId, String> {
        let (id, encoded) = self.prepare_request(method, params)?;
        self.write_request(&encoded)?;
        Ok(id)
    }

    fn submission_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<CodexRequestId, String> {
        let (id, encoded) = self.prepare_request(method, params)?;
        if let Err(error) = self.write_request(&encoded) {
            self.events.push_back(WorkerEvent::Failed(format!(
                "Codex prompt delivery is unknown: {error}"
            )));
        }
        Ok(id)
    }

    fn prepare_request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<(CodexRequestId, Vec<u8>), String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "Codex worker request id overflow".to_owned())?;
        let id = CodexRequestId::Number(self.next_id);
        let encoded = encode_request(&id, method, params)?;
        Ok((id, encoded))
    }

    fn write_request(&mut self, encoded: &[u8]) -> Result<(), String> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| "Codex worker input is closed".to_owned())?;
        writer
            .write_all(encoded)
            .and_then(|()| writer.flush())
            .map_err(|error| format!("write Codex worker request: {error}"))
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

const STEER_CLIENT_ID_PREFIX: &str = "farcaster-steer-";
const QUEUE_CLIENT_ID_PREFIX: &str = "farcaster-queue-";
const HANDOFF_CLIENT_ID_PREFIX: &str = "farcaster-handoff-";
const NORMAL_CLIENT_ID_PREFIX: &str = "farcaster-normal-";

fn queue_submission_id(result: &Value) -> Result<String, String> {
    result
        .pointer("/queuedSubmission/id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "decode Codex queue acknowledgement: missing queued submission id".into())
}

fn steer_rejected_by_turn_race(error: &super::contract::CodexRpcError) -> bool {
    error.message == "no active turn to steer"
        || (error.message.starts_with("expected active turn id `")
            && error.message.contains("` but found `")
            && error.message.ends_with('`'))
}

fn codex_input_delivery(item: &Value) -> Option<WorkerActivity> {
    if item.get("type").and_then(Value::as_str) != Some("userMessage") {
        return None;
    }
    let client_id = item.get("clientId").and_then(Value::as_str)?;
    let mode = if client_id.starts_with(STEER_CLIENT_ID_PREFIX) {
        WorkerSendMode::Steer
    } else if client_id.starts_with(QUEUE_CLIENT_ID_PREFIX) {
        WorkerSendMode::Queue
    } else if client_id.starts_with(HANDOFF_CLIENT_ID_PREFIX) {
        WorkerSendMode::Steer
    } else if client_id.starts_with(NORMAL_CLIENT_ID_PREFIX) {
        WorkerSendMode::Prompt
    } else {
        return None;
    };
    let content = super::catalog::user_content(item.get("content"));
    let message = content
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let images: Vec<crate::protocol::PromptImage> = content
        .into_iter()
        .filter_map(|part| serde_json::from_value(part).ok())
        .collect();
    if images.is_empty() {
        (!message.is_empty()).then_some(WorkerActivity::InputDelivered { mode, message })
    } else {
        Some(WorkerActivity::InputDeliveredWithImages {
            mode,
            message,
            images,
        })
    }
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

fn decode_codex_goal(value: &Value) -> Result<Option<SessionGoal>, String> {
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| format!("decode thread goal: {error}"))
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
        input: input.saturating_sub(cache_read.saturating_add(cache_write)),
        output: value
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read,
        cache_write,
    }
}

fn codex_telemetry(method: &str, params: &Value) -> Option<WorkerActivity> {
    match method {
        "mcpServer/startupStatus/updated" => Some(WorkerActivity::ServiceStatusChanged {
            name: params.get("name")?.as_str()?.to_owned(),
            status: params.get("status")?.as_str()?.to_owned(),
            error: params
                .get("error")
                .filter(|value| !value.is_null())
                .cloned(),
            failure_reason: params
                .get("failureReason")
                .filter(|value| !value.is_null())
                .cloned(),
        }),
        "account/rateLimits/updated" => Some(WorkerActivity::RateLimitsChanged {
            limits: params.get("rateLimits")?.clone(),
        }),
        _ => None,
    }
}

fn codex_child_event_outcome(item: &Value) -> Option<crate::agents::ChildSessionOutcome> {
    match item.get("kind").and_then(Value::as_str) {
        Some("completed") => Some(crate::agents::ChildSessionOutcome::Complete),
        Some("failed") => Some(crate::agents::ChildSessionOutcome::Failed),
        Some("interrupted") => Some(crate::agents::ChildSessionOutcome::Incomplete),
        _ => None,
    }
}

fn codex_child_thread_outcome(thread: &Value) -> Option<crate::agents::ChildSessionOutcome> {
    if thread.pointer("/status/type").and_then(Value::as_str) == Some("systemError") {
        return Some(crate::agents::ChildSessionOutcome::Failed);
    }
    match thread
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| turns.last())
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
    {
        Some("completed") => Some(crate::agents::ChildSessionOutcome::Complete),
        Some("failed") => Some(crate::agents::ChildSessionOutcome::Failed),
        Some("interrupted") => Some(crate::agents::ChildSessionOutcome::Incomplete),
        _ => None,
    }
}

fn codex_notification_is_for_thread(method: &str, params: &Value, thread_id: &str) -> bool {
    match params.get("threadId").and_then(Value::as_str) {
        Some(reported) if reported != thread_id => false,
        _ if matches!(method, "warning" | "configWarning") => {
            zlog::warn!("Codex app-server {method}: {params}");
            false
        }
        _ if method == "remoteControl/status/changed" => false,
        Some(_) => true,
        None if method == "thread/started" => false,
        None => {
            log_bad_codex_notification(method, params, "notification is missing threadId");
            false
        }
    }
}

fn log_bad_codex_notification(method: &str, params: &Value, reason: &str) {
    zlog::warn!(
        "Codex notification was not mapped correctly ({reason}): method={method} params={params}"
    );
}

fn codex_error_message(params: &Value) -> Option<String> {
    let message = params.pointer("/error/message").and_then(Value::as_str)?;
    (!message.trim().is_empty()).then(|| message.to_owned())
}

fn codex_turn_failure(turn_error: Option<String>) -> String {
    match turn_error {
        Some(message) => format!("Codex worker turn failed: {message}"),
        None => "Codex worker turn failed".into(),
    }
}

fn is_codex_approval_request(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "execCommandApproval"
            | "applyPatchApproval"
    )
}

fn codex_passive_item(item: &Value) -> bool {
    item.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| {
            matches!(
                kind,
                "userMessage"
                    | "agentMessage"
                    | "plan"
                    | "reasoning"
                    | "contextCompaction"
                    | "compacted"
                    | "enteredReviewMode"
                    | "exitedReviewMode"
                    | "hookPrompt"
            )
        })
}

fn codex_tool_review_started(params: &Value) -> Option<(WorkerActivity, WorkerActivity)> {
    let id = params.get("targetItemId")?.as_str()?;
    let action = params.get("action")?;
    let kind = action.get("type")?.as_str()?;
    let (name, args) = match kind {
        "command" => (
            CommonTool::Bash.name().to_owned(),
            json!({
                "command": action.get("command").cloned().unwrap_or(Value::Null),
                "cwd": action.get("cwd").cloned().unwrap_or(Value::Null),
            }),
        ),
        _ => (kind.to_owned(), action.clone()),
    };
    Some((
        WorkerActivity::ToolStarted {
            id: id.to_owned(),
            name,
            args,
            metadata: tool::metadata(action, kind),
        },
        WorkerActivity::ToolReviewChanged {
            id: id.to_owned(),
            state: ToolReviewState::Reviewing,
            detail: None,
        },
    ))
}

fn codex_tool_review_completed(params: &Value) -> Option<(WorkerActivity, Option<WorkerActivity>)> {
    let review = params.get("review")?;
    let approved = review.get("status").and_then(Value::as_str) == Some("approved");
    let mut summary = Vec::new();
    if let Some(risk) = review.get("riskLevel").and_then(Value::as_str) {
        summary.push(format!("Risk: {risk}"));
    }
    if let Some(authorization) = review.get("userAuthorization").and_then(Value::as_str) {
        summary.push(format!("Authorization: {authorization}"));
    }
    if let Some(rationale) = review.get("rationale").and_then(Value::as_str) {
        summary.push(rationale.to_owned());
    }
    let id = params.get("targetItemId")?.as_str()?.to_owned();
    let state = if approved {
        ToolReviewState::Approved
    } else {
        ToolReviewState::Blocked
    };
    let finished = (!approved).then(|| WorkerActivity::ToolFinished {
        id: id.clone(),
        result: json!([]),
        is_error: true,
    });
    Some((
        WorkerActivity::ToolReviewChanged {
            id,
            state,
            detail: (!summary.is_empty()).then(|| summary.join("\n")),
        },
        finished,
    ))
}

fn codex_tool_start(params: &Value) -> Option<WorkerActivity> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !tool::is_tool_kind(kind) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let projection = tool::project(item, kind);
    Some(WorkerActivity::ToolStarted {
        id: id.to_owned(),
        name: projection.name,
        args: projection.args,
        metadata: projection.metadata,
    })
}

fn codex_tool_metadata_changed(params: &Value) -> Option<WorkerActivity> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !tool::is_tool_kind(kind) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let projection = tool::project(item, kind);
    Some(WorkerActivity::ToolMetadataChanged {
        id: id.to_owned(),
        args: Some(projection.args),
        metadata: projection.metadata,
    })
}

fn codex_tool_end(params: &Value) -> Option<WorkerActivity> {
    let item = params.get("item")?;
    let kind = item.get("type")?.as_str()?;
    if !tool::is_tool_kind(kind) {
        return None;
    }
    let id = item.get("id")?.as_str()?;
    let failed = item
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "failed" | "declined"));
    let result = if let Some(content) = item.pointer("/result/content").and_then(Value::as_array) {
        Value::Array(content.clone())
    } else if failed
        && let Some(error) = item
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| item.get("error").and_then(Value::as_str))
    {
        json!([{"type": "text", "text": error}])
    } else if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
        json!([{"type": "text", "text": output}])
    } else if let Some(output) = item.get("result").or_else(|| item.get("output")) {
        json!([{
            "type": "text",
            "text": output.as_str().map(str::to_owned).unwrap_or_else(|| output.to_string()),
        }])
    } else if kind == "commandExecution" && item.get("aggregatedOutput").is_none_or(Value::is_null)
    {
        // Commands may complete without output, including when output was streamed.
        json!([{"type": "text", "text": ""}])
    } else if kind == "webSearch" {
        json!([{"type": "text", "text": tool::web_search_query(item).unwrap_or_default()}])
    } else if kind == "fileChange" && !failed {
        json!([{"type": "text", "text": "Applied patch"}])
    } else if kind == "imageView" {
        json!([{
            "type": "text",
            "text": item.get("path").and_then(Value::as_str).unwrap_or_default(),
        }])
    } else if kind == "sleep" {
        json!([{
            "type": "text",
            "text": format!("Waited {}", tool::wait_duration(item)),
        }])
    } else if kind == "subAgentActivity" {
        json!([{"type": "text", "text": tool::subagent_summary(item)}])
    } else if kind == "collabAgentToolCall" {
        json!([{
            "type": "text",
            "text": item
                .get("agentsStates")
                .map(Value::to_string)
                .unwrap_or_else(|| item.get("status").map(Value::to_string).unwrap_or_default()),
        }])
    } else {
        zlog::warn!("Codex tool completion had no mappable result: {item}");
        json!([])
    };
    Some(WorkerActivity::ToolFinished {
        id: id.to_owned(),
        result,
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
#[path = "worker_tests.rs"]
mod tests;
