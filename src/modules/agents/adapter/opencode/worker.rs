use std::{
    collections::{HashMap, HashSet, VecDeque},
    process::Stdio,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::{
    server::OpenCodeServerProcess,
    tool::{normalize_opencode_tool, opencode_tool_metadata},
};
use crate::{
    agents::{
        AgentLaunchConfig, TokenUsage, WorkerActivity, WorkerActivityState, WorkerContext,
        WorkerEvent, WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession,
        WorkerSessionFactory, WorkerUsage,
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
        if launch.ephemeral {
            return Err("OpenCode does not expose ephemeral inference".into());
        }
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("OpenCode worker provider and model must be supplied together".into());
        }
        let mut command = self.command.clone();
        command.access_mode = launch.access_mode;
        command.app_proxy = launch.app_proxy.clone();
        let mut prepared = command.command(&launch.project)?;
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared()
            .issue_as_with_access(
                &launch.project,
                crate::modules::agents::core::CallerProfile {
                    backend: "opencode2".into(),
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
        let password = worker_password()?;
        configure_opencode_server(&mut prepared, launch.access_mode)?;
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
                let parent_id = launch.parent_worker_id.as_deref().and_then(|id| {
                    crate::agents::CallerRegistry::shared().native_parent_session(id, "opencode2")
                });
                client.create_session(
                    &launch.project.to_string_lossy(),
                    parent_id.as_deref(),
                    selected_model,
                )?
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
            WorkerContext::Resume { session_locator } => {
                let session = client.get_session(&session_locator)?;
                if session.id != session_locator {
                    return Err("OpenCode returned a different child session on resume".into());
                }
                session
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
            effort_catalog: HashMap::new(),
            access_mode: launch.access_mode,
            incoming,
            reasoning_started: false,
            text_streams: HashMap::new(),
            reasoning_streams: HashMap::new(),
            usage: OpenCodeUsageTracker::default(),
            context_window: 0,
            pending_inputs: HashMap::new(),
            pending_deliveries: HashMap::new(),
            delivered_awaiting_execution: HashSet::new(),
            active_tools: HashMap::new(),
            generation: 0,
            completions: None,
            turn_active: false,
            steering_interrupts: 0,
            ignore_execution_events: false,
            abort_waiting_for_start: false,
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
    let result = load_main_metadata(&mut server.client(), &project.to_string_lossy()).and_then(
        |mut metadata| {
            complete_model_catalog(command, project, &mut metadata)?;
            Ok(metadata)
        },
    );
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
    let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue_with_access(
        &launch.project,
        crate::modules::agents::core::CallerProfile {
            backend: "opencode2".into(),
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
        command.access_mode,
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
    let mut metadata = load_main_metadata(&mut client, &launch.project.to_string_lossy())?;
    if let Err(error) = complete_model_catalog(command, &launch.project, &mut metadata) {
        zlog::warn!("OpenCode started without a refreshed model catalog: {error}");
    }
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
            let session = client.get_session(&session_id)?;
            if session.id != session_id {
                return Err("OpenCode returned a different session on resume".into());
            }
            session
        }
        crate::agents::SessionStart::Fork(_) => {
            let session_id = main_session::launch_session_locator(launch)
                .ok_or_else(|| "OpenCode fork requires a session id".to_owned())?;
            client.fork_session(&session_id, None)?
        }
    };
    metadata.session_name = session.title;
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
            effort_catalog: effort_catalog(&metadata),
            access_mode: command.access_mode,
            incoming,
            reasoning_started: false,
            text_streams: HashMap::new(),
            reasoning_streams: HashMap::new(),
            usage: OpenCodeUsageTracker::default(),
            context_window,
            pending_inputs: HashMap::new(),
            pending_deliveries: HashMap::new(),
            delivered_awaiting_execution: HashSet::new(),
            active_tools: HashMap::new(),
            generation: 0,
            completions: None,
            turn_active: false,
            steering_interrupts: 0,
            ignore_execution_events: false,
            abort_waiting_for_start: false,
            wake: launch.wake.clone(),
            pending: VecDeque::new(),
        }),
        session_id,
        metadata,
    ))
}

fn complete_model_catalog(
    command: &AgentLaunchConfig,
    project: &std::path::Path,
    metadata: &mut crate::modules::agents::adapter::main_session::MainSessionMetadata,
) -> Result<(), String> {
    if !metadata.models.is_empty() {
        return Ok(());
    }
    let mut prepared = command.command(project)?;
    let output = prepared
        .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
        .arg("models")
        .output()
        .map_err(|error| format!("run OpenCode model catalog fallback: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "OpenCode model API and CLI fallback were unavailable (exit {})",
            output.status.code().unwrap_or(-1)
        ));
    }
    metadata.models = models_from_cli(&String::from_utf8_lossy(&output.stdout));
    if metadata.models.is_empty() {
        Err("OpenCode model API and CLI fallback returned no models".into())
    } else {
        Ok(())
    }
}

fn models_from_cli(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|reference| {
            let (provider, id) = reference.trim().split_once('/')?;
            (!provider.is_empty() && !id.is_empty()).then(|| {
                json!({
                    "id": id,
                    "name": id,
                    "provider": provider,
                    "contextWindow": 0,
                    "reasoning": true,
                })
            })
        })
        .collect()
}

fn load_main_metadata(
    client: &mut super::client::OpenCodeClient<super::transport::OpenCodeTcpTransport>,
    directory: &str,
) -> Result<crate::modules::agents::adapter::main_session::MainSessionMetadata, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let model_rows = loop {
        let model_response = client.models(directory)?;
        let rows = model_response
            .as_array()
            .or_else(|| model_response.get("data").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();
        if !rows.is_empty() || Instant::now() >= deadline {
            break rows;
        }
        thread::sleep(Duration::from_millis(50));
    };
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
            let model_efforts = model_variant_efforts(model);
            let efforts_known = model.get("variants").is_some();
            for effort in &model_efforts {
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
                "efforts": efforts_known.then_some(model_efforts),
            }))
        })
        .collect::<Vec<_>>();
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
            ..Default::default()
        },
    )
}

fn model_variant_efforts(model: &Value) -> Vec<String> {
    model
        .get("variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|variant| variant.as_str().or_else(|| variant.get("id")?.as_str()))
        .map(str::to_owned)
        .collect()
}

fn effort_catalog(
    metadata: &crate::modules::agents::adapter::main_session::MainSessionMetadata,
) -> HashMap<(String, String), Vec<String>> {
    metadata
        .models
        .iter()
        .filter_map(|model| {
            let provider = model.get("provider")?.as_str()?;
            let id = model.get("id")?.as_str()?;
            let efforts = model
                .get("efforts")?
                .as_array()?
                .iter()
                .filter_map(|effort| effort.as_str())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            Some(((provider.to_owned(), id.to_owned()), efforts))
        })
        .collect()
}

fn variant_for_model(effort: Option<&str>, known: Option<&Vec<String>>) -> Option<String> {
    let effort = effort?;
    match known {
        Some(efforts) => efforts
            .iter()
            .any(|candidate| candidate == effort)
            .then(|| effort.to_owned()),
        None => Some(effort.to_owned()),
    }
}

#[derive(Default)]
struct OpenCodeUsageTracker {
    session: TokenUsage,
    context: TokenUsage,
}

impl OpenCodeUsageTracker {
    fn step_ended(&mut self, turn: TokenUsage) -> (TokenUsage, TokenUsage) {
        self.context = turn;
        self.session = self.session.saturating_add(turn);
        (turn, self.session)
    }

    fn session_total(&mut self, total: TokenUsage) -> (TokenUsage, TokenUsage) {
        self.session = total;
        (self.context, self.session)
    }
}

#[derive(Default)]
struct ActiveOpenCodeTool {
    name: String,
    input: String,
    args: Option<Value>,
    native: Value,
    started: bool,
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

#[derive(Clone)]
struct PendingOpenCodeDelivery {
    submission_id: Option<String>,
    mode: WorkerSendMode,
    message: String,
    images: Vec<crate::protocol::PromptImage>,
    clears_abort_barrier: bool,
}

struct OpenCodeWorkerSession {
    caller_identity: crate::modules::agents::core::CallerIdentity,
    server: OpenCodeServerProcess,
    session_id: String,
    provider: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    effort_catalog: HashMap<(String, String), Vec<String>>,
    access_mode: crate::agents::HarnessAccessMode,
    incoming: mpsc::Receiver<Result<super::contract::OpenCodeEvent, String>>,
    reasoning_started: bool,
    text_streams: HashMap<String, String>,
    reasoning_streams: HashMap<String, String>,
    usage: OpenCodeUsageTracker,
    context_window: u64,
    pending_inputs: HashMap<String, PendingOpenCodeInput>,
    pending_deliveries: HashMap<String, PendingOpenCodeDelivery>,
    delivered_awaiting_execution: HashSet<String>,
    active_tools: HashMap<String, ActiveOpenCodeTool>,
    generation: u64,
    completions: Option<mpsc::Receiver<(u64, Result<String, String>)>>,
    turn_active: bool,
    steering_interrupts: usize,
    ignore_execution_events: bool,
    abort_waiting_for_start: bool,
    wake: Option<thread::Thread>,
    pending: VecDeque<WorkerEvent>,
}

impl OpenCodeWorkerSession {
    fn send_prompt(
        &mut self,
        submission_id: Option<String>,
        native_id: Option<String>,
        message: String,
        mode: WorkerSendMode,
        files: Vec<super::contract::OpenCodeFileInput>,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, super::contract::OpenCodePromptDispatchError> {
        let delivery = match mode {
            WorkerSendMode::Prompt | WorkerSendMode::Queue => {
                super::contract::OpenCodeDelivery::Queue
            }
            WorkerSendMode::Steer => super::contract::OpenCodeDelivery::Steer,
        };
        let clears_abort_barrier = self.ignore_execution_events;
        let pending = PendingOpenCodeDelivery {
            submission_id,
            mode,
            message,
            images,
            clears_abort_barrier,
        };
        if let Some(native_id) = &native_id {
            self.pending_deliveries
                .insert(native_id.clone(), pending.clone());
        }
        let admission = match self.server.client().prompt(
            &self.session_id,
            native_id.as_deref(),
            &pending.message,
            files,
            delivery,
        ) {
            Ok(admission) => admission,
            Err(super::contract::OpenCodePromptDispatchError::Unsent(error)) => {
                if let Some(native_id) = &native_id {
                    self.pending_deliveries.remove(native_id);
                }
                return Err(super::contract::OpenCodePromptDispatchError::Unsent(error));
            }
            Err(error @ super::contract::OpenCodePromptDispatchError::Unknown(_)) => {
                return Err(error);
            }
        };
        self.record_prompt_admission(admission, native_id.as_deref(), pending)
            .map_err(super::contract::OpenCodePromptDispatchError::Unknown)?;
        Ok(true)
    }

    fn next_internal_prompt_id() -> String {
        format!("msg_farcaster_{}", uuid::Uuid::new_v4())
    }

    fn record_prompt_admission(
        &mut self,
        admission: super::contract::OpenCodePromptAdmission,
        native_id: Option<&str>,
        delivery: PendingOpenCodeDelivery,
    ) -> Result<(), String> {
        let was_active = self.turn_active;
        let expected_delivery = match delivery.mode {
            WorkerSendMode::Steer => "steer",
            WorkerSendMode::Prompt | WorkerSendMode::Queue => "queue",
        };
        if admission.session_id != self.session_id
            || admission.id.is_empty()
            || native_id.is_some_and(|native_id| admission.id != native_id)
            || admission.delivery != expected_delivery
        {
            return Err("OpenCode returned an invalid prompt admission receipt".into());
        }
        self.pending_deliveries.insert(admission.id, delivery);
        self.caller_identity
            .set_activity(WorkerActivityState::Working);
        if !was_active {
            self.reasoning_started = false;
            self.clear_streams();
        }
        self.generation = self.generation.saturating_add(1);
        self.completions = None;
        self.turn_active = true;
        if !was_active {
            self.pending.push_back(WorkerEvent::Started);
        }
        Ok(())
    }

    fn finish_turn(&mut self) {
        self.abort_waiting_for_start = false;
        self.delivered_awaiting_execution.clear();
        self.turn_active = false;
        self.completions = None;
        self.clear_streams();
        self.caller_identity.set_activity(WorkerActivityState::Idle);
    }

    fn finish_abort(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.steering_interrupts = 0;
        self.ignore_execution_events = true;
        self.finish_turn();
        self.pending.push_front(WorkerEvent::Settled {
            output: String::new(),
        });
    }

    fn delivered_input(&mut self, native_id: &str) -> Option<WorkerEvent> {
        let delivery = self.pending_deliveries.remove(native_id)?;
        self.delivered_awaiting_execution
            .insert(native_id.to_owned());
        if delivery.clears_abort_barrier {
            self.ignore_execution_events = false;
        }
        let activity = match (delivery.submission_id, delivery.images.is_empty()) {
            (Some(submission_id), true) => WorkerActivity::SubmittedInputDelivered {
                submission_id,
                mode: delivery.mode,
                message: delivery.message,
            },
            (Some(submission_id), false) => WorkerActivity::SubmittedInputDeliveredWithImages {
                submission_id,
                mode: delivery.mode,
                message: delivery.message,
                images: delivery.images,
            },
            (None, true) => WorkerActivity::InputDelivered {
                mode: delivery.mode,
                message: delivery.message,
            },
            (None, false) => WorkerActivity::InputDeliveredWithImages {
                mode: delivery.mode,
                message: delivery.message,
                images: delivery.images,
            },
        };
        Some(WorkerEvent::Activity(activity))
    }

    fn clear_streams(&mut self) {
        self.active_tools.clear();
        self.text_streams.clear();
        self.reasoning_streams.clear();
    }

    fn fetch_completed_context(&mut self) -> Result<(), String> {
        if self.completions.is_some() {
            return Ok(());
        }
        let generation = self.generation;
        let session_id = self.session_id.clone();
        let mut client = self.server.client();
        let (sender, receiver) = mpsc::channel();
        let wake = self.wake.clone();
        thread::Builder::new()
            .name(format!("opencode-context-{session_id}"))
            .spawn(move || {
                let result = client
                    .context(&session_id)
                    .map(|context| final_assistant_text(&context));
                let _ = send_and_wake(&sender, (generation, result), wake.as_ref());
            })
            .map_err(|error| format!("read completed OpenCode context: {error}"))?;
        self.completions = Some(receiver);
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
            let Some(reported_event_type) = event.event.as_deref() else {
                log_bad_opencode_event(&event, "missing event type");
                continue;
            };
            match opencode_child_activity(&event, &self.session_id, |id| {
                self.server.client().get_session(id)
            }) {
                Ok(Some(activity)) => return Some(WorkerEvent::Activity(activity)),
                Ok(None) => {}
                Err(error) => {
                    zlog::warn!("Failed to read OpenCode child session: {error}");
                }
            }
            if !opencode_event_is_for_session(&event, reported_event_type, &self.session_id) {
                continue;
            }
            let event_type = unversioned_opencode_event_type(reported_event_type);
            if self.ignore_execution_events && opencode_event_belongs_to_execution(event_type) {
                continue;
            }
            match event_type {
                "session.execution.started" => {
                    if self.abort_waiting_for_start {
                        match self.server.client().interrupt(&self.session_id, false) {
                            Ok(true) => {
                                self.finish_abort();
                                return self.pending.pop_front();
                            }
                            Ok(false) => continue,
                            Err(error) => return Some(WorkerEvent::Failed(error)),
                        }
                    }
                    self.delivered_awaiting_execution.clear();
                    if !self.turn_active {
                        self.turn_active = true;
                        self.caller_identity
                            .set_activity(WorkerActivityState::Working);
                        return Some(WorkerEvent::Started);
                    }
                }
                "session.execution.succeeded" => {
                    if self.turn_active
                        && let Err(error) = self.fetch_completed_context()
                    {
                        self.finish_turn();
                        return Some(WorkerEvent::Failed(error));
                    }
                }
                "session.execution.interrupted" => {
                    if self.steering_interrupts > 0 {
                        self.steering_interrupts -= 1;
                        self.generation = self.generation.saturating_add(1);
                        self.completions = None;
                        continue;
                    }
                    if self.turn_active {
                        self.finish_turn();
                        return Some(WorkerEvent::Settled {
                            output: String::new(),
                        });
                    }
                }
                "session.execution.failed" => {
                    if self.turn_active {
                        self.finish_turn();
                        return Some(WorkerEvent::Failed(opencode_event_error(
                            &event.data,
                            "OpenCode execution failed",
                        )));
                    }
                }
                "session.updated" | "session.renamed" => {
                    if let Some(title) = opencode_session_title(&event.data) {
                        return Some(WorkerEvent::Activity(WorkerActivity::TitleChanged(title)));
                    }
                }
                "session.inbox.delivered" => {
                    let Some(id) = event
                        .data
                        .get("inboxID")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    else {
                        log_bad_opencode_event(&event, "inbox delivery is missing inboxID");
                        continue;
                    };
                    if let Some(delivered) = self.delivered_input(&id) {
                        return Some(delivered);
                    }
                }
                "session.inbox.cancelled" => {
                    if let Some(id) = event.data.get("inboxID").and_then(Value::as_str) {
                        self.pending_deliveries.remove(id);
                    }
                }
                "session.next.prompted" => {
                    let id = event
                        .data
                        .get("messageID")
                        .and_then(Value::as_str)
                        .or_else(|| event.data.get("inboxID").and_then(Value::as_str));
                    if let Some(id) = id
                        && let Some(delivered) = self.delivered_input(id)
                    {
                        return Some(delivered);
                    }
                }
                "session.text.started" | "session.next.text.started" => {
                    if let Some(key) = opencode_part_key(&event.data) {
                        self.text_streams.entry(key).or_default();
                    }
                }
                "session.text.delta" | "session.next.text.delta" => {
                    let Some(delta) = event.data.get("delta").and_then(Value::as_str) else {
                        log_bad_opencode_event(&event, "text delta is missing delta");
                        continue;
                    };
                    if let Some(key) = opencode_part_key(&event.data) {
                        self.text_streams.entry(key).or_default().push_str(delta);
                    }
                    return Some(WorkerEvent::Activity(WorkerActivity::TextDelta {
                        content_index: usize::from(self.reasoning_started),
                        delta: delta.to_owned(),
                    }));
                }
                "session.text.ended" | "session.next.text.ended" => {
                    let Some(text) = event.data.get("text").and_then(Value::as_str) else {
                        log_bad_opencode_event(&event, "text end is missing text");
                        continue;
                    };
                    let streamed = opencode_part_key(&event.data)
                        .and_then(|key| self.text_streams.remove(&key))
                        .unwrap_or_default();
                    let Some(delta) = completed_opencode_delta(&streamed, text) else {
                        continue;
                    };
                    return Some(WorkerEvent::Activity(WorkerActivity::TextDelta {
                        content_index: usize::from(self.reasoning_started),
                        delta,
                    }));
                }
                "session.reasoning.started" | "session.next.reasoning.started" => {
                    if let Some(key) = opencode_part_key(&event.data) {
                        self.reasoning_streams.entry(key).or_default();
                    }
                    self.reasoning_started = true;
                    return Some(WorkerEvent::Activity(WorkerActivity::ThinkingStarted {
                        content_index: 0,
                    }));
                }
                "session.reasoning.delta" | "session.next.reasoning.delta" => {
                    let Some(delta) = event.data.get("delta").and_then(Value::as_str) else {
                        log_bad_opencode_event(&event, "reasoning delta is missing delta");
                        continue;
                    };
                    if let Some(key) = opencode_part_key(&event.data) {
                        self.reasoning_streams
                            .entry(key)
                            .or_default()
                            .push_str(delta);
                    }
                    self.reasoning_started = true;
                    return Some(WorkerEvent::Activity(WorkerActivity::ThinkingDelta {
                        content_index: 0,
                        delta: delta.to_owned(),
                    }));
                }
                "session.reasoning.ended" | "session.next.reasoning.ended" => {
                    let Some(text) = event.data.get("text").and_then(Value::as_str) else {
                        log_bad_opencode_event(&event, "reasoning end is missing text");
                        continue;
                    };
                    let streamed = opencode_part_key(&event.data)
                        .and_then(|key| self.reasoning_streams.remove(&key))
                        .unwrap_or_default();
                    let Some(delta) = completed_opencode_delta(&streamed, text) else {
                        continue;
                    };
                    self.reasoning_started = true;
                    return Some(WorkerEvent::Activity(WorkerActivity::ThinkingDelta {
                        content_index: 0,
                        delta,
                    }));
                }
                "session.tool.input.started" | "session.next.tool.input.started" => {
                    let Some(id) = opencode_tool_id(&event.data).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "tool input start is missing call id");
                        continue;
                    };
                    let (name, _) = normalize_opencode_tool(
                        opencode_tool_name(&event.data).unwrap_or("tool"),
                        &Value::Null,
                    );
                    self.active_tools.insert(
                        id,
                        ActiveOpenCodeTool {
                            name,
                            native: event.data.clone(),
                            ..Default::default()
                        },
                    );
                }
                "session.tool.input.delta" | "session.next.tool.input.delta" => {
                    let Some(id) = opencode_tool_id(&event.data).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "tool input delta is missing call id");
                        continue;
                    };
                    let Some(delta) = event.data.get("delta").and_then(Value::as_str) else {
                        log_bad_opencode_event(&event, "tool input delta is missing delta");
                        continue;
                    };
                    self.active_tools
                        .entry(id)
                        .or_default()
                        .input
                        .push_str(delta);
                }
                "session.tool.input.ended" | "session.next.tool.input.ended" => {
                    let Some(id) = opencode_tool_id(&event.data).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "tool input end is missing call id");
                        continue;
                    };
                    let tool = self.active_tools.entry(id.clone()).or_default();
                    merge_opencode_native(&mut tool.native, &event.data);
                    let text = event
                        .data
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or(&tool.input);
                    let args = match serde_json::from_str(text) {
                        Ok(args) => args,
                        Err(error) => {
                            log_bad_opencode_event(
                                &event,
                                &format!("tool input end contains invalid JSON: {error}"),
                            );
                            continue;
                        }
                    };
                    let (name, args) = normalize_opencode_tool(&tool.name, &args);
                    let metadata = opencode_tool_metadata(&name, &args, tool.native.clone());
                    tool.name.clone_from(&name);
                    tool.args = Some(args.clone());
                    tool.started = true;
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolStarted {
                        id,
                        name,
                        args,
                        metadata,
                    }));
                }
                "session.tool.called" | "session.next.tool.called" => {
                    let Some(id) = opencode_tool_id(&event.data).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "tool call is missing call id");
                        continue;
                    };
                    let tool = self.active_tools.entry(id.clone()).or_default();
                    merge_opencode_native(&mut tool.native, &event.data);
                    let reported_name = opencode_tool_name(&event.data)
                        .filter(|name| !name.is_empty())
                        .or_else(|| (!tool.name.is_empty()).then_some(tool.name.as_str()))
                        .unwrap_or("tool");
                    let (name, args) = normalize_opencode_tool(
                        reported_name,
                        event.data.get("input").unwrap_or(&Value::Null),
                    );
                    let metadata = opencode_tool_metadata(&name, &args, tool.native.clone());
                    tool.name.clone_from(&name);
                    tool.args = Some(args.clone());
                    if std::mem::replace(&mut tool.started, true) {
                        return Some(WorkerEvent::Activity(WorkerActivity::ToolMetadataChanged {
                            id,
                            args: Some(args),
                            metadata,
                        }));
                    }
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolStarted {
                        id,
                        name,
                        args,
                        metadata,
                    }));
                }
                "session.tool.progress" | "session.next.tool.progress" => {
                    let Some(id) = opencode_tool_id(&event.data).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "tool progress is missing call id");
                        continue;
                    };
                    let tool = self.active_tools.entry(id.clone()).or_default();
                    merge_opencode_native(&mut tool.native, &event.data);
                    let name = tool.name.as_str();
                    let args = event
                        .data
                        .get("input")
                        .map(|input| normalize_opencode_tool(name, input).1)
                        .or_else(|| tool.args.clone());
                    if let Some(args) = &args {
                        tool.args = Some(args.clone());
                    }
                    let metadata = opencode_tool_metadata(
                        name,
                        args.as_ref().unwrap_or(&Value::Null),
                        tool.native.clone(),
                    );
                    self.pending
                        .push_back(WorkerEvent::Activity(WorkerActivity::ToolUpdated {
                            id: id.clone(),
                            content: json!([{
                                "type": "text",
                                "text": event.data.get("metadata").map(Value::to_string).unwrap_or_default(),
                            }]),
                        }));
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolMetadataChanged {
                        id,
                        args,
                        metadata,
                    }));
                }
                "session.step.started" | "session.next.step.started" => {
                    self.delivered_awaiting_execution.clear();
                    return Some(WorkerEvent::Activity(WorkerActivity::TurnStarted));
                }
                "session.step.ended" | "session.next.step.ended" => {
                    let Some(turn) = opencode_event_usage(&event.data) else {
                        log_bad_opencode_event(&event, "step end is missing token usage");
                        continue;
                    };
                    let (turn, session) = self.usage.step_ended(turn);
                    return Some(WorkerEvent::Activity(WorkerActivity::Usage(WorkerUsage {
                        turn,
                        session,
                        context_window: self.context_window,
                    })));
                }
                "session.usage.updated" | "session.usage.recorded" => {
                    let Some(total) = opencode_event_usage(&event.data) else {
                        log_bad_opencode_event(&event, "usage event is missing token usage");
                        continue;
                    };
                    let (turn, session) = self.usage.session_total(total);
                    return Some(WorkerEvent::Activity(WorkerActivity::Usage(WorkerUsage {
                        turn,
                        session,
                        context_window: self.context_window,
                    })));
                }
                "session.tool.success"
                | "session.tool.failed"
                | "session.next.tool.success"
                | "session.next.tool.failed" => {
                    let Some(id) = opencode_tool_id(&event.data).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "tool completion is missing call id");
                        continue;
                    };
                    let failed = event_type.ends_with("tool.failed");
                    let finished = WorkerEvent::Activity(WorkerActivity::ToolFinished {
                        id: id.clone(),
                        result: opencode_tool_result(&event.data, failed),
                        is_error: failed,
                    });
                    let Some(mut tool) = self.active_tools.remove(&id) else {
                        return Some(finished);
                    };
                    merge_opencode_native(&mut tool.native, &event.data);
                    let args = event
                        .data
                        .get("input")
                        .map(|input| normalize_opencode_tool(&tool.name, input).1)
                        .or(tool.args);
                    let metadata = opencode_tool_metadata(
                        &tool.name,
                        args.as_ref().unwrap_or(&Value::Null),
                        tool.native,
                    );
                    self.pending.push_back(finished);
                    return Some(WorkerEvent::Activity(WorkerActivity::ToolMetadataChanged {
                        id,
                        args,
                        metadata,
                    }));
                }
                "session.retry.scheduled" => {
                    return Some(WorkerEvent::Activity(
                        WorkerActivity::ServiceStatusChanged {
                            name: "opencode".into(),
                            status: "retrying".into(),
                            error: event.data.get("error").cloned(),
                            failure_reason: event.data.get("reason").cloned(),
                        },
                    ));
                }
                "session.step.failed" => {
                    return Some(WorkerEvent::Activity(
                        WorkerActivity::ServiceStatusChanged {
                            name: "opencode".into(),
                            status: "step_failed".into(),
                            error: event.data.get("error").cloned(),
                            failure_reason: event.data.get("reason").cloned(),
                        },
                    ));
                }
                "session.compaction.started" | "session.next.compaction.started" => {
                    return Some(WorkerEvent::Activity(WorkerActivity::CompactionStarted));
                }
                "session.compaction.ended" | "session.next.compaction.ended" => {
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
                "session.compaction.failed" => {
                    return Some(WorkerEvent::Activity(WorkerActivity::CompactionFinished {
                        aborted: false,
                        error: Some(opencode_event_error(
                            &event.data,
                            "OpenCode compaction failed",
                        )),
                    }));
                }
                "form.created" => {
                    let Some(form) = event.data.get("form") else {
                        log_bad_opencode_event(&event, "form event is missing form");
                        continue;
                    };
                    let Some(id) = form.get("id").and_then(Value::as_str).map(str::to_owned) else {
                        log_bad_opencode_event(&event, "form is missing id");
                        continue;
                    };
                    let title = form
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("OpenCode question")
                        .to_owned();
                    let Some(fields) = form.get("fields").and_then(Value::as_array) else {
                        log_bad_opencode_event(&event, "form has no fields");
                        continue;
                    };
                    let Some(field) = fields.first() else {
                        log_bad_opencode_event(&event, "form has no fields");
                        continue;
                    };
                    if fields.len() > 1 {
                        log_bad_opencode_event(&event, "only the first form field can be mapped");
                    }
                    let Some(key) = field.get("key").and_then(Value::as_str).map(str::to_owned)
                    else {
                        log_bad_opencode_event(&event, "form field is missing key");
                        continue;
                    };
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
                "session.next.prompt.admitted"
                | "session.step.streamed"
                | "session.next.compaction.delta" => {}
                _ => log_bad_opencode_event(&event, "unmapped same-session event"),
            }
        }
    }
}

impl WorkerSession for OpenCodeWorkerSession {
    fn tracks_prompt_delivery(&self, _mode: WorkerSendMode) -> bool {
        true
    }

    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        let native_id = Self::next_internal_prompt_id();
        match self.send_prompt(
            Some(native_id.clone()),
            Some(native_id.clone()),
            message,
            mode,
            Vec::new(),
            Vec::new(),
        ) {
            Ok(_) => Ok(()),
            Err(super::contract::OpenCodePromptDispatchError::Unsent(error)) => Err(error),
            Err(super::contract::OpenCodePromptDispatchError::Unknown(error)) => {
                self.pending.push_back(WorkerEvent::PromptDeliveryUnknown {
                    submission_id: native_id,
                    error,
                });
                Ok(())
            }
        }
    }

    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        let images = images
            .into_iter()
            .map(crate::protocol::PromptImage::into_inline)
            .collect::<Result<Vec<_>, _>>()?;
        let files = images
            .iter()
            .enumerate()
            .map(|(index, image)| super::contract::OpenCodeFileInput {
                uri: format!("data:{};base64,{}", image.mime_type, image.data),
                name: Some(format!("image-{}", index + 1)),
                description: None,
            })
            .collect();
        let native_id = Self::next_internal_prompt_id();
        match self.send_prompt(
            Some(native_id.clone()),
            Some(native_id.clone()),
            message,
            mode,
            files,
            images,
        ) {
            Ok(_) => Ok(()),
            Err(super::contract::OpenCodePromptDispatchError::Unsent(error)) => Err(error),
            Err(super::contract::OpenCodePromptDispatchError::Unknown(error)) => {
                self.pending.push_back(WorkerEvent::PromptDeliveryUnknown {
                    submission_id: native_id,
                    error,
                });
                Ok(())
            }
        }
    }

    fn submit_prompt(
        &mut self,
        id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        let images = images
            .into_iter()
            .map(crate::protocol::PromptImage::into_inline)
            .collect::<Result<Vec<_>, _>>()?;
        let files = images
            .iter()
            .enumerate()
            .map(|(index, image)| super::contract::OpenCodeFileInput {
                uri: format!("data:{};base64,{}", image.mime_type, image.data),
                name: Some(format!("image-{}", index + 1)),
                description: None,
            })
            .collect();
        match self.send_prompt(
            Some(id.clone()),
            Some(format!("msg_{id}")),
            message,
            mode,
            files,
            images,
        ) {
            Ok(accepted) => Ok(accepted),
            Err(super::contract::OpenCodePromptDispatchError::Unsent(error)) => Err(error),
            Err(super::contract::OpenCodePromptDispatchError::Unknown(error)) => {
                self.pending.push_back(WorkerEvent::PromptDeliveryUnknown {
                    submission_id: id,
                    error,
                });
                Ok(false)
            }
        }
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
        let mut client = self.server.client();
        let interrupted = client.interrupt(&self.session_id, false)?;
        let mut delivery_may_start = !self.delivered_awaiting_execution.is_empty();
        for native_id in self.pending_deliveries.keys().cloned().collect::<Vec<_>>() {
            match client.cancel_inbox(&self.session_id, &native_id) {
                Ok(true) => {
                    self.pending_deliveries.remove(&native_id);
                }
                Ok(false) => delivery_may_start = true,
                Err(error) => return Err(error),
            }
        }
        if interrupted || !delivery_may_start {
            self.finish_abort();
        } else {
            self.abort_waiting_for_start = true;
            self.ignore_execution_events = false;
        }
        Ok(())
    }

    fn apply_steering(&mut self) -> Result<(), String> {
        let mut client = self.server.client();
        let native_ids = self
            .pending_deliveries
            .iter()
            .filter_map(|(id, delivery)| {
                (delivery.mode == WorkerSendMode::Queue).then(|| id.clone())
            })
            .collect::<Vec<_>>();
        let (interrupted, errors) = promote_followups_and_interrupt(
            &mut client,
            &self.session_id,
            native_ids.iter().map(String::as_str),
        );
        if interrupted == Some(true) {
            // The interrupted execution resumes on the server. Settling here
            // would clear the composer's pending steering and follow-ups.
            self.steering_interrupts += 1;
            self.generation = self.generation.saturating_add(1);
            self.completions = None;
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("apply OpenCode steering: {}", errors.join("; ")))
        }
    }

    fn compact(&mut self) -> Result<(), String> {
        self.server.client().compact_session(&self.session_id)
    }

    fn rename(&mut self, name: &str) -> Result<(), String> {
        self.server.client().rename_session(&self.session_id, name)
    }

    fn select_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        let known = self
            .effort_catalog
            .get(&(provider.to_owned(), model.to_owned()));
        let variant = variant_for_model(self.effort.as_deref(), known);
        self.caller_identity.select_model(provider, model);
        self.server
            .client()
            .select_model(&self.session_id, provider, model, variant.as_deref())?;
        self.provider = Some(provider.to_owned());
        self.model = Some(model.to_owned());
        if variant.is_none() {
            self.effort = None;
        }
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
            let activity = if self.turn_active {
                WorkerActivityState::Working
            } else {
                WorkerActivityState::Idle
            };
            let mode = WorkerSendMode::for_peer(activity).expect("OpenCode is ready for delivery");
            return Some(match self.send_peer_message(&message, mode) {
                Ok(()) => {
                    self.pending.push_back(WorkerEvent::Started);
                    WorkerEvent::Activity(WorkerActivity::PeerInputDelivered { message })
                }
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
        self.finish_turn();
        Some(match completion.1 {
            Ok(output) => WorkerEvent::Settled { output },
            Err(error) => WorkerEvent::Failed(error),
        })
    }

    fn close(&mut self) -> Result<(), String> {
        self.server.terminate()
    }
}

fn opencode_child_activity(
    event: &super::contract::OpenCodeEvent,
    parent_id: &str,
    lookup_session: impl FnOnce(&str) -> Result<super::contract::OpenCodeSession, String>,
) -> Result<Option<WorkerActivity>, String> {
    let Some(event_type) = event.event.as_deref() else {
        return Ok(None);
    };
    let is_running = match unversioned_opencode_event_type(event_type) {
        "session.execution.started" => true,
        "session.execution.succeeded"
        | "session.execution.failed"
        | "session.execution.interrupted" => false,
        _ => return Ok(None),
    };
    let Some(id) = event.data.get("sessionID").and_then(Value::as_str) else {
        return Ok(None);
    };
    if id.is_empty() || id == parent_id {
        return Ok(None);
    }
    // The event stream includes other sessions. Resolve the native parent before
    // publishing a child, including when we attached after that child was created.
    let child = lookup_session(id)?;
    Ok((child.parent_id.as_deref() == Some(parent_id)).then_some(
        WorkerActivity::ChildSessionsChanged {
            id: child.id,
            title: child.title,
            is_running,
            outcome: None,
        },
    ))
}

fn opencode_event_is_for_session(
    event: &super::contract::OpenCodeEvent,
    event_type: &str,
    session_id: &str,
) -> bool {
    let reported = event
        .data
        .get("sessionID")
        .and_then(Value::as_str)
        .or_else(|| {
            event
                .data
                .pointer("/form/sessionID")
                .and_then(Value::as_str)
        });
    match reported {
        Some(reported) => reported == session_id,
        None => {
            if event_type.starts_with("session.") || event_type == "form.created" {
                log_bad_opencode_event(event, "session event is missing sessionID");
            }
            false
        }
    }
}

fn opencode_session_title(data: &Value) -> Option<String> {
    data.get("title")
        .or_else(|| data.pointer("/info/title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
}

fn unversioned_opencode_event_type(event_type: &str) -> &str {
    let Some((base, version)) = event_type.rsplit_once('.') else {
        return event_type;
    };
    if version.bytes().all(|byte| byte.is_ascii_digit()) {
        base
    } else {
        event_type
    }
}

fn opencode_part_key(data: &Value) -> Option<String> {
    let message = data.get("assistantMessageID")?.as_str()?;
    let ordinal = data.get("ordinal").and_then(Value::as_u64).unwrap_or(0);
    Some(format!("{message}:{ordinal}"))
}

fn completed_opencode_delta(streamed: &str, completed: &str) -> Option<String> {
    if streamed.is_empty() {
        return (!completed.is_empty()).then(|| completed.to_owned());
    }
    completed
        .strip_prefix(streamed)
        .filter(|suffix| !suffix.is_empty())
        .map(str::to_owned)
}

fn opencode_event_error(data: &Value, fallback: &str) -> String {
    data.pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| data.get("error").and_then(Value::as_str))
        .or_else(|| data.get("message").and_then(Value::as_str))
        .unwrap_or(fallback)
        .to_owned()
}

fn opencode_event_usage(data: &Value) -> Option<TokenUsage> {
    let usage = data.get("tokens").or_else(|| data.get("usage"))?;
    Some(TokenUsage {
        input: opencode_token(usage.get("input")),
        output: opencode_token(usage.get("output"))
            .saturating_add(opencode_token(usage.get("reasoning"))),
        cache_read: opencode_token(usage.pointer("/cache/read")),
        cache_write: opencode_token(usage.pointer("/cache/write")),
    })
}

fn opencode_token(value: Option<&Value>) -> u64 {
    value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|value| value.max(0.0) as u64))
        })
        .unwrap_or(0)
}

fn opencode_tool_result(data: &Value, failed: bool) -> Value {
    if failed {
        let error = data
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| data.get("error").and_then(Value::as_str));
        if error.is_none() {
            zlog::warn!("OpenCode failed tool event is missing its error: {data}");
        }
        return json!([{"type": "text", "text": error.unwrap_or("OpenCode tool failed")}]);
    }
    if let Some(content) = data.get("content").and_then(Value::as_array) {
        return Value::Array(content.clone());
    }
    if let Some(content) = data.pointer("/result/content").and_then(Value::as_array) {
        return Value::Array(content.clone());
    }
    let output = data
        .get("result")
        .or_else(|| data.get("structured"))
        .or_else(|| data.get("output"));
    output.map_or_else(
        || {
            zlog::warn!("OpenCode successful tool event has no mappable result: {data}");
            json!([])
        },
        |output| {
            json!([{
                "type": "text",
                "text": output.as_str().map(str::to_owned).unwrap_or_else(|| output.to_string()),
            }])
        },
    )
}

fn merge_opencode_native(current: &mut Value, update: &Value) {
    merge_json(current, update.clone());
}

fn log_bad_opencode_event(event: &super::contract::OpenCodeEvent, reason: &str) {
    zlog::warn!("OpenCode event was not mapped correctly ({reason}): {event:?}");
}

fn opencode_event_belongs_to_execution(event_type: &str) -> bool {
    event_type.starts_with("session.execution.")
        || event_type.starts_with("session.text.")
        || event_type.starts_with("session.reasoning.")
        || event_type.starts_with("session.tool.")
        || event_type.starts_with("session.step.")
        || event_type.starts_with("session.next.text.")
        || event_type.starts_with("session.next.reasoning.")
        || event_type.starts_with("session.next.tool.")
        || event_type.starts_with("session.next.step.")
}

fn promote_followups_and_interrupt<'a, T: super::contract::OpenCodeHttpTransport>(
    client: &mut super::client::OpenCodeClient<T>,
    session_id: &str,
    native_ids: impl IntoIterator<Item = &'a str>,
) -> (Option<bool>, Vec<String>) {
    let mut errors = Vec::new();
    for native_id in native_ids {
        if let Err(error) = client.steer_inbox(session_id, native_id) {
            errors.push(format!("{native_id}: {error}"));
        }
    }
    let interrupted = match client.interrupt(session_id, true) {
        Ok(interrupted) => Some(interrupted),
        Err(error) => {
            errors.push(format!("interrupt OpenCode execution: {error}"));
            None
        }
    };
    (interrupted, errors)
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
    command
        .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
        .args(["serve", "--stdio", "--print-logs"]);
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
#[path = "worker_tests.rs"]
mod tests;
