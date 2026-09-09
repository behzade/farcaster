use std::{
    collections::{HashMap, VecDeque},
    process::{Child, Stdio},
    thread,
};

use serde_json::{Value, json};

use super::{
    AcpProfile,
    connection::AcpConnection,
    events::{AcpInbound, AcpRequestId},
    translate::{
        ConfigIds, commands_from_update, commands_from_value, content_text, find_permission_option,
        is_acceptance, merge_tool_metadata, normalize_content, normalize_tool_name, tool_args,
        tool_result, usage_update,
    },
};
use crate::{
    agents::{
        AgentLaunchConfig, HarnessAccessMode, PeerMessage, ToolMetadata, WorkerActivity,
        WorkerActivityState, WorkerEvent, WorkerInput, WorkerInputResponse, WorkerLaunch,
        WorkerSendMode, WorkerSession, WorkerSessionFactory,
    },
    modules::agents::adapter::{child_stderr, farcaster_mcp, main_session},
};

#[derive(Clone)]
pub(in crate::modules::agents::adapter) struct AcpWorkerFactory {
    command: AgentLaunchConfig,
    profile: AcpProfile,
}

impl AcpWorkerFactory {
    pub(in crate::modules::agents::adapter) fn new(
        command: AgentLaunchConfig,
        profile: AcpProfile,
    ) -> Self {
        Self { command, profile }
    }
}

impl WorkerSessionFactory for AcpWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.ephemeral {
            return Err(format!(
                "{} does not expose ephemeral inference",
                self.profile.name
            ));
        }
        if launch.provider.is_some() != launch.model.is_some() {
            return Err(format!(
                "{} worker provider and model must be supplied together",
                self.profile.name
            ));
        }
        if launch
            .provider
            .as_deref()
            .is_some_and(|provider| provider != self.profile.backend)
        {
            return Err(format!(
                "{} worker model must use provider {}",
                self.profile.name, self.profile.backend
            ));
        }
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared()
            .issue_as(
                &launch.project,
                crate::modules::agents::core::CallerProfile {
                    backend: self.profile.backend.into(),
                    provider: launch.provider.clone(),
                    model: launch.model.clone(),
                    effort: launch.effort.clone(),
                },
                None,
                launch.worker_id.clone(),
                launch.worker_name.clone(),
                launch.parent_worker_id.clone(),
            )?
            .with_slot(launch.slot.clone());
        if matches!(launch.context, crate::agents::WorkerContext::Session { .. }) {
            return Err(format!(
                "{} does not advertise ACP session fork for inherited workers",
                self.profile.name
            ));
        }
        let (mut session, _, _) = spawn_session(
            &self.command,
            &self.profile,
            &launch.project,
            None,
            None,
            None,
        )?;
        if let Some(model) = launch.model.as_deref() {
            session.select_model(
                launch
                    .provider
                    .as_deref()
                    .expect("provider checked with model"),
                model,
            )?;
        }
        if let Some(effort) = launch.effort.as_deref()
            && session.config_ids.effort.is_some()
        {
            session.select_effort(effort)?;
        }
        caller_identity.bind(session.session_id.clone());
        session.events.push_back(WorkerEvent::SessionChanged {
            locator: session.session_id.clone(),
        });
        Ok(Box::new(session.with_identity(caller_identity)))
    }
}

pub(in crate::modules::agents::adapter) fn spawn_main(
    command: &AgentLaunchConfig,
    profile: &AcpProfile,
    launch: &crate::agents::SessionLaunch,
) -> Result<super::MainSession, String> {
    let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue(
        &launch.project,
        crate::modules::agents::core::CallerProfile {
            backend: profile.backend.into(),
            provider: None,
            model: None,
            effort: None,
        },
        launch.wake.clone(),
    );
    let resume = match &launch.start {
        crate::agents::SessionStart::New => None,
        crate::agents::SessionStart::Resume(_) => Some(
            main_session::launch_session_locator(launch)
                .ok_or_else(|| format!("{} resume requires a session id", profile.name))?,
        ),
        crate::agents::SessionStart::Fork(_) => {
            return Err(format!("{} does not expose ACP session fork", profile.name));
        }
    };
    let (session, metadata, history) = spawn_session(
        command,
        profile,
        &launch.project,
        resume.as_deref(),
        Some(caller_identity.token()),
        launch.wake.clone(),
    )?;
    let locator = session.session_id.clone();
    caller_identity.bind(locator.clone());
    Ok((
        Box::new(session.with_identity(caller_identity)),
        locator,
        metadata,
        history,
    ))
}

fn spawn_session(
    command: &AgentLaunchConfig,
    profile: &AcpProfile,
    project: &std::path::Path,
    resume: Option<&str>,
    caller_token: Option<&str>,
    wake: Option<thread::Thread>,
) -> Result<
    (
        AcpWorkerSession,
        super::super::main_session::MainSessionMetadata,
        Option<crate::agents::DiscoveredHistory>,
    ),
    String,
> {
    let mut prepared = command.command(project)?;
    configure_command(&mut prepared, profile, command.access_mode)?;
    let mut child = prepared
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start {} ACP agent: {error}", profile.name))?;
    child_stderr::capture(&mut child, "acp-agent")?;
    let AcpSetup {
        connection,
        session_id,
        mut metadata,
        config_ids,
        features,
        history,
    } = match setup_connection(&mut child, profile, project, resume, caller_token, wake) {
        Ok(setup) => setup,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{} ACP setup failed: {error}. Check the installed runtime and its authentication before retrying.",
                profile.name
            ));
        }
    };
    if let Some(mode) = profile.permission_mode(command.access_mode) {
        let (method, params) = if let Some(id) = &config_ids.mode {
            (
                "session/set_config_option",
                json!({"sessionId":session_id,"configId":id,"value":mode}),
            )
        } else {
            (
                "session/set_mode",
                json!({"sessionId":session_id,"modeId":mode}),
            )
        };
        if let Err(error) = connection.request_blocking(method, params) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{} could not apply access mode: {error}",
                profile.name
            ));
        }
        if let Some(index) = metadata
            .modes
            .iter()
            .position(|entry| entry.get("id").and_then(Value::as_str) == Some(mode))
        {
            metadata.modes.swap(0, index);
        }
    }
    Ok((
        AcpWorkerSession {
            profile: profile.clone(),
            child,
            connection,
            session_id,
            current_prompt: None,
            pending_steers: HashMap::new(),
            queued_prompts: VecDeque::new(),
            output: String::new(),
            thought_started: false,
            pending_inputs: HashMap::new(),
            tool_states: HashMap::new(),
            peer_messages: VecDeque::new(),
            events: VecDeque::new(),
            config_ids,
            features,
            caller_identity: None,
        },
        metadata,
        history,
    ))
}

struct AcpSetup {
    connection: AcpConnection,
    session_id: String,
    metadata: super::super::main_session::MainSessionMetadata,
    config_ids: ConfigIds,
    features: AcpFeatures,
    history: Option<crate::agents::DiscoveredHistory>,
}

fn setup_connection(
    child: &mut Child,
    profile: &AcpProfile,
    project: &std::path::Path,
    resume: Option<&str>,
    caller_token: Option<&str>,
    wake: Option<thread::Thread>,
) -> Result<AcpSetup, String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("{} ACP stdin must be piped", profile.name))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{} ACP stdout must be piped", profile.name))?;
    let mut connection = AcpConnection::new(
        blocking::Unblock::new(stdout),
        blocking::Unblock::new(stdin),
        wake,
    )?;
    let initialized = connection.initialize(profile)?;
    let features = AcpFeatures::from_initialize(&initialized);
    let params = json!({
        "cwd": project.to_string_lossy(),
        "mcpServers": acp_mcp_servers(caller_token),
    });
    let response = if let Some(session_id) = resume {
        connection.request_blocking(
            profile.resume_method,
            merge(params, "sessionId", Value::String(session_id.into())),
        )?
    } else {
        connection.request_blocking("session/new", params)?
    };
    let session_id = response
        .get("sessionId")
        .and_then(Value::as_str)
        .or(resume)
        .ok_or_else(|| format!("{} ACP agent did not provide a session id", profile.name))?
        .to_owned();
    let (mut metadata, config_ids) =
        super::configuration::metadata(profile, &response, connection.model_catalog(profile)?);
    let queued = connection.drain_queued()?;
    if let Some(commands) = queued
        .iter()
        .filter_map(|message| commands_from_update(message, &session_id))
        .next_back()
    {
        metadata.commands = commands;
    }
    let history = resume.is_some().then(|| {
        let mut history = super::catalog::discovered_history(
            profile,
            queued.iter().cloned(),
            &response,
            &session_id,
        );
        if let Some(model) = &config_ids.selected_model {
            history.model = Some((profile.backend.into(), model.clone()));
        }
        history
    });
    if resume.is_none() {
        connection.restore_queued(queued);
    }
    Ok(AcpSetup {
        connection,
        session_id,
        metadata,
        config_ids,
        features,
        history,
    })
}

fn merge(mut object: Value, key: &str, value: Value) -> Value {
    object[key] = value;
    object
}

fn prompt_content(
    message: &str,
    images: Vec<crate::protocol::PromptImage>,
) -> Result<Vec<Value>, String> {
    let images = images
        .into_iter()
        .map(crate::protocol::PromptImage::into_inline)
        .collect::<Result<Vec<_>, _>>()?;
    let mut prompt = vec![json!({"type": "text", "text": message})];
    prompt.extend(
        images
            .into_iter()
            .map(|image| json!({"type": "image", "mimeType": image.mime_type, "data": image.data})),
    );
    Ok(prompt)
}

pub(in crate::modules::agents::adapter) fn configure_command(
    command: &mut std::process::Command,
    profile: &AcpProfile,
    access_mode: HarnessAccessMode,
) -> Result<(), String> {
    if profile.backend == "antigravity-acp" {
        super::super::antigravity::configure(command)?;
    }
    if access_mode == HarnessAccessMode::Full
        && let Some(argument) = profile.force_argument
    {
        command.arg(argument);
    }
    command.args(profile.arguments);
    Ok(())
}

fn acp_mcp_servers(caller_token: Option<&str>) -> Vec<Value> {
    if !farcaster_mcp::enabled() {
        return Vec::new();
    }
    caller_token
        .map(|token| {
            vec![json!({
                "type": "http",
                "name": "farcaster",
                "url": farcaster_mcp::URL,
                "headers": [{"name": farcaster_mcp::CALLER_HEADER, "value": token}],
            })]
        })
        .unwrap_or_default()
}

struct AcpFeatures {
    steering: bool,
    close: bool,
}

impl AcpFeatures {
    fn from_initialize(value: &Value) -> Self {
        Self {
            steering: value
                .pointer("/_meta/steering/supported")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            close: value
                .pointer("/agentCapabilities/sessionCapabilities/close")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

struct PendingInput {
    request: AcpRequestId,
    kind: PendingInputKind,
}

enum PendingInputKind {
    Permission {
        option_ids: HashMap<String, String>,
        allow_option: Option<String>,
        reject_option: Option<String>,
    },
    CursorQuestions {
        title: String,
        questions: Vec<super::cursor_extension::CursorQuestion>,
        index: usize,
        option_index: usize,
        selected: Vec<String>,
        answers: Vec<Value>,
    },
    CursorPlan,
}

#[derive(Clone, Default)]
struct ToolState {
    name: String,
    args: Value,
    metadata: ToolMetadata,
    started: bool,
    finished: bool,
}

struct AcpWorkerSession {
    profile: AcpProfile,
    child: Child,
    connection: AcpConnection,
    session_id: String,
    current_prompt: Option<AcpRequestId>,
    pending_steers: HashMap<AcpRequestId, (String, Vec<crate::protocol::PromptImage>)>,
    queued_prompts: VecDeque<(WorkerSendMode, String, Vec<crate::protocol::PromptImage>)>,
    output: String,
    thought_started: bool,
    pending_inputs: HashMap<String, PendingInput>,
    tool_states: HashMap<String, ToolState>,
    peer_messages: VecDeque<PeerMessage>,
    events: VecDeque<WorkerEvent>,
    config_ids: ConfigIds,
    features: AcpFeatures,
    caller_identity: Option<crate::modules::agents::core::CallerIdentity>,
}

impl AcpWorkerSession {
    fn with_identity(mut self, identity: crate::modules::agents::core::CallerIdentity) -> Self {
        self.caller_identity = Some(identity);
        self
    }

    fn request(&mut self, method: &str, params: Value) -> Result<AcpRequestId, String> {
        self.connection.send_request(method, params)
    }

    fn start_prompt_request(
        &mut self,
        message: &str,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        let prompt = prompt_content(message, images)?;
        self.output.clear();
        self.thought_started = false;
        self.tool_states.clear();
        let id = self.request(
            "session/prompt",
            json!({"sessionId": self.session_id, "prompt": prompt}),
        )?;
        self.current_prompt = Some(id);
        if let Some(identity) = &self.caller_identity {
            identity.set_activity(WorkerActivityState::Working);
        }
        Ok(())
    }

    fn request_and_wait(&mut self, method: &str, params: Value) -> Result<(), String> {
        let response = self.connection.request_blocking(method, params)?;
        if response.get("configOptions").is_some() {
            self.refresh_configuration(&response);
        }
        Ok(())
    }

    fn refresh_configuration(&mut self, response: &Value) {
        let (metadata, ids) = super::configuration::metadata(
            &self.profile,
            response,
            self.config_ids.catalog.clone(),
        );
        let selected_model = metadata
            .models
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == ids.selected_model.as_deref())
            .cloned();
        let current_value = |id: Option<&str>| {
            response
                .get("configOptions")
                .and_then(Value::as_array)
                .and_then(|options| {
                    options
                        .iter()
                        .find(|option| option.get("id").and_then(Value::as_str) == id)
                })
                .and_then(|option| option.get("currentValue"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let selected_effort = current_value(ids.effort.as_deref());
        let selected_mode = current_value(ids.mode.as_deref());
        self.config_ids = ids;
        self.events
            .push_back(WorkerEvent::Activity(WorkerActivity::ServiceTierChanged {
                selected: metadata.service_tier,
                options: metadata.service_tiers,
            }));
        self.events.push_back(WorkerEvent::Activity(
            WorkerActivity::ConfigurationChanged {
                models: metadata.models,
                efforts: metadata.efforts,
                modes: metadata.modes,
                selected_model,
                selected_effort,
            },
        ));
        if let Some(mode) = selected_mode {
            self.events
                .push_back(WorkerEvent::Activity(WorkerActivity::ModeChanged(mode)));
        }
    }

    fn update(&mut self, params: Value) -> Option<WorkerEvent> {
        let session_id = params.get("sessionId").and_then(Value::as_str);
        if session_id != Some(&self.session_id) {
            if session_id.is_none() {
                log_bad_acp_message(
                    self.profile.name,
                    "session/update",
                    &params,
                    "missing sessionId",
                );
            }
            return None;
        }
        let update = params.get("update");
        let update_type = update
            .and_then(|update| update.get("sessionUpdate"))
            .and_then(Value::as_str);
        let (Some(update), Some(update_type)) = (update, update_type) else {
            log_bad_acp_message(
                self.profile.name,
                "session/update",
                &params,
                "malformed update",
            );
            return None;
        };
        match update_type {
            "current_mode_update" => update
                .get("currentModeId")
                .and_then(Value::as_str)
                .map(|mode| WorkerEvent::Activity(WorkerActivity::ModeChanged(mode.into()))),
            "session_info_update" => update
                .get("title")
                .and_then(Value::as_str)
                .map(|title| WorkerEvent::Activity(WorkerActivity::TitleChanged(title.into()))),
            "plan" => {
                let todos = update
                    .get("entries")?
                    .as_array()?
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        json!({"id": index.to_string(), "content": entry.get("content"),
                        "status": entry.get("status"), "priority": entry.get("priority")})
                    })
                    .collect::<Vec<_>>();
                let summary = todos
                    .iter()
                    .map(|todo| {
                        format!(
                            "{}: {}",
                            todo.get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("pending"),
                            todo.get("content")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let id = format!(
                    "plan-{}",
                    self.current_prompt
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                );
                // Each plan snapshot replaces the previous result in the same
                // tool row, including revisions after an earlier completion.
                if let Some(state) = self.tool_states.get_mut(&id) {
                    state.finished = false;
                }
                self.tool_update(
                    &json!({"toolCallId":id,"title":"Update plan","kind":"other",
                    "status":"completed","rawInput":{"todos":todos},
                    "content":[{"type":"content","content":{"type":"text","text":summary}}]}),
                )
            }
            "agent_message_chunk" => {
                let text = content_text(update.get("content")?)?;
                self.output.push_str(&text);
                Some(WorkerEvent::Activity(WorkerActivity::TextDelta {
                    content_index: usize::from(self.thought_started),
                    delta: text,
                }))
            }
            "agent_thought_chunk" => {
                let text = content_text(update.get("content")?)?;
                self.thought_started = true;
                Some(WorkerEvent::Activity(WorkerActivity::ThinkingDelta {
                    content_index: 0,
                    delta: text,
                }))
            }
            "tool_call" | "tool_call_update" => self.tool_update(update),
            "usage_update" => usage_update(update)
                .map(WorkerActivity::Usage)
                .map(WorkerEvent::Activity),
            "available_commands_update" => commands_from_value(update).map(|commands| {
                WorkerEvent::Activity(WorkerActivity::CommandsChanged { commands })
            }),
            "config_option_update" => {
                update.get("configOptions")?.as_array()?;
                self.refresh_configuration(update);
                self.events.pop_front()
            }
            _ => {
                log_bad_acp_message(
                    self.profile.name,
                    "session/update",
                    &params,
                    "unmapped same-session update",
                );
                None
            }
        }
    }

    fn tool_update(&mut self, update: &Value) -> Option<WorkerEvent> {
        let id = update.get("toolCallId")?.as_str()?.to_owned();
        let mut emitted = VecDeque::new();
        {
            let state = self.tool_states.entry(id.clone()).or_default();
            let previous_metadata = state.metadata.clone();
            let previous_args = state.args.clone();
            merge_tool_metadata(&mut state.metadata, update);
            state.args = tool_args(&state.metadata);

            if !state.started {
                state.started = true;
                let title = state.metadata.title.as_deref().unwrap_or("tool");
                let native = state.metadata.native.as_ref().unwrap_or(update);
                state.name = normalize_tool_name(native, title);
                emitted.push_back(WorkerEvent::Activity(WorkerActivity::ToolStarted {
                    id: id.clone(),
                    name: state.name.clone(),
                    args: state.args.clone(),
                    metadata: state.metadata.clone(),
                }));
            } else if state.metadata != previous_metadata || state.args != previous_args {
                emitted.push_back(WorkerEvent::Activity(WorkerActivity::ToolMetadataChanged {
                    id: id.clone(),
                    args: (state.args != previous_args).then(|| state.args.clone()),
                    metadata: state.metadata.clone(),
                }));
            }

            let status = update.get("status").and_then(Value::as_str);
            if matches!(status, Some("completed" | "failed")) && !state.finished {
                state.finished = true;
                emitted.push_back(WorkerEvent::Activity(WorkerActivity::ToolFinished {
                    id: id.clone(),
                    result: tool_result(&state.metadata, update),
                    is_error: status == Some("failed"),
                }));
            } else if !state.finished
                && let Some(content) = update.get("content").or_else(|| update.get("rawOutput"))
            {
                emitted.push_back(WorkerEvent::Activity(WorkerActivity::ToolUpdated {
                    id: id.clone(),
                    content: normalize_content(content),
                }));
            }
        }
        let first = emitted.pop_front();
        self.events.extend(emitted);
        first
    }

    fn permission_request(&mut self, id: &AcpRequestId, params: &Value) -> Option<WorkerEvent> {
        if params.get("sessionId").and_then(Value::as_str) != Some(&self.session_id) {
            return None;
        }
        let options = params.get("options")?.as_array()?;
        let input_id = id.to_string();
        let choices = options
            .iter()
            .filter_map(|option| {
                let id = option
                    .get("optionId")
                    .or_else(|| option.get("id"))
                    .and_then(Value::as_str)?;
                let label = option
                    .get("name")
                    .or_else(|| option.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(id);
                Some((label.to_owned(), id.to_owned()))
            })
            .collect::<Vec<_>>();
        let labels = choices
            .iter()
            .map(|(label, _)| label.clone())
            .collect::<Vec<_>>();
        let option_ids = choices.into_iter().collect();
        let allow_option = find_permission_option(options, true);
        let reject_option = find_permission_option(options, false);
        self.pending_inputs.insert(
            input_id.clone(),
            PendingInput {
                request: id.clone(),
                kind: PendingInputKind::Permission {
                    option_ids,
                    allow_option,
                    reject_option,
                },
            },
        );
        let title = params
            .pointer("/toolCall/title")
            .or_else(|| params.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("Agent requests permission");
        Some(WorkerEvent::NeedsInput(WorkerInput {
            id: input_id,
            prompt: title.into(),
            options: if labels.is_empty() {
                vec!["Allow".into(), "Decline".into()]
            } else {
                labels
            },
            secret: false,
        }))
    }

    fn cursor_request(
        &mut self,
        id: &AcpRequestId,
        method: &str,
        params: &Value,
    ) -> Option<WorkerEvent> {
        if self.profile.backend != "cursor-cli" {
            return None;
        }
        let (input, kind) = match super::cursor_extension::request(method, params)? {
            super::cursor_extension::CursorRequest::Questions { title, questions } => {
                let input =
                    super::cursor_extension::question_input(id, &title, questions.first()?, 0, 0);
                (
                    input,
                    PendingInputKind::CursorQuestions {
                        title,
                        questions,
                        index: 0,
                        option_index: 0,
                        selected: Vec::new(),
                        answers: Vec::new(),
                    },
                )
            }
            super::cursor_extension::CursorRequest::Plan { prompt } => (
                super::cursor_extension::plan_input(id, prompt),
                PendingInputKind::CursorPlan,
            ),
        };
        self.pending_inputs.insert(
            input.id.clone(),
            PendingInput {
                request: id.clone(),
                kind,
            },
        );
        Some(WorkerEvent::NeedsInput(input))
    }

    fn cursor_notification(&mut self, method: &str, params: &Value) -> Option<WorkerEvent> {
        if self.profile.backend != "cursor-cli" {
            return None;
        }
        let (started, finished) = super::cursor_extension::notification(method, params)?;
        self.events.push_back(finished);
        Some(started)
    }

    fn reject_request(&mut self, id: &AcpRequestId) -> Result<(), String> {
        self.connection
            .respond(id, json!({"outcome": {"outcome": "cancelled"}}))
    }
}

impl WorkerSession for AcpWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_with_images(message, mode, Vec::new())
    }

    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        if mode == WorkerSendMode::Steer && self.features.steering {
            if self.current_prompt.is_none() {
                self.start_prompt_request(&message, images)?;
                self.events.push_back(WorkerEvent::Started);
                return Ok(());
            }
            let id = self.request(
                "_session/steering",
                json!({
                    "sessionId": self.session_id,
                    "prompt": prompt_content(&message, images.clone())?,
                    "_meta": {"steering": {"idleBehavior": "promptRequired"}},
                }),
            )?;
            self.pending_steers.insert(id, (message, images));
            return Ok(());
        }
        if self.current_prompt.is_some() {
            if matches!(mode, WorkerSendMode::Queue | WorkerSendMode::Steer) {
                // Keep the requested mode for delivery acknowledgements even
                // when an ACP agent needs to defer steering to its next turn.
                self.queued_prompts.push_back((mode, message, images));
                return Ok(());
            }
            return Err(format!(
                "{} ACP session is already working",
                self.profile.name
            ));
        }
        self.start_prompt_request(&message, images)?;
        self.events.push_back(WorkerEvent::Started);
        Ok(())
    }

    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        let pending = self
            .pending_inputs
            .remove(&response.id)
            .ok_or_else(|| format!("unknown ACP interaction: {}", response.id))?;
        let PendingInput { request, kind } = pending;
        let result = match kind {
            PendingInputKind::Permission {
                option_ids,
                allow_option,
                reject_option,
            } => {
                if response.cancel {
                    json!({"outcome": {"outcome": "cancelled"}})
                } else {
                    let value = response.value.as_deref().unwrap_or_default();
                    let option_id = option_ids.get(value).cloned().or_else(|| {
                        if is_acceptance(value) {
                            allow_option
                        } else {
                            reject_option
                        }
                    });
                    let option_id = option_id.ok_or_else(|| {
                        "ACP permission response has no matching option".to_owned()
                    })?;
                    json!({"outcome": {"outcome": "selected", "optionId": option_id}})
                }
            }
            PendingInputKind::CursorPlan => {
                let outcome = if response.cancel {
                    json!({"outcome": "cancelled"})
                } else if is_acceptance(response.value.as_deref().unwrap_or_default()) {
                    json!({"outcome": "accepted"})
                } else {
                    json!({"outcome": "rejected"})
                };
                json!({"outcome": outcome})
            }
            PendingInputKind::CursorQuestions {
                title,
                questions,
                index,
                option_index,
                mut selected,
                mut answers,
            } => {
                if response.cancel {
                    json!({"outcome": {"outcome": "cancelled"}})
                } else {
                    let question = questions
                        .get(index)
                        .ok_or_else(|| "Cursor question index is invalid".to_owned())?;
                    let value = response.value.as_deref().unwrap_or_default();
                    if question.allow_multiple {
                        if is_acceptance(value) {
                            selected.push(question.options[option_index].1.clone());
                        }
                        let next_option = option_index + 1;
                        if next_option < question.options.len() {
                            let input = super::cursor_extension::question_input(
                                &request,
                                &title,
                                question,
                                index,
                                next_option,
                            );
                            self.pending_inputs.insert(
                                input.id.clone(),
                                PendingInput {
                                    request,
                                    kind: PendingInputKind::CursorQuestions {
                                        title,
                                        questions,
                                        index,
                                        option_index: next_option,
                                        selected,
                                        answers,
                                    },
                                },
                            );
                            self.events.push_back(WorkerEvent::NeedsInput(input));
                            return Ok(());
                        }
                    } else {
                        selected.push(
                            question
                                .options
                                .iter()
                                .find(|(label, id)| label == value || id == value)
                                .map(|(_, id)| id.clone())
                                .ok_or_else(|| {
                                    "Cursor question response has no matching option".to_owned()
                                })?,
                        );
                    }
                    answers.push(json!({
                        "questionId": question.id,
                        "selectedOptionIds": selected,
                    }));
                    let next = index + 1;
                    if let Some(question) = questions.get(next) {
                        let input = super::cursor_extension::question_input(
                            &request, &title, question, next, 0,
                        );
                        self.pending_inputs.insert(
                            input.id.clone(),
                            PendingInput {
                                request,
                                kind: PendingInputKind::CursorQuestions {
                                    title,
                                    questions,
                                    index: next,
                                    option_index: 0,
                                    selected: Vec::new(),
                                    answers,
                                },
                            },
                        );
                        self.events.push_back(WorkerEvent::NeedsInput(input));
                        return Ok(());
                    }
                    json!({"outcome": {"outcome": "answered", "answers": answers}})
                }
            }
        };
        self.connection.respond(&request, result)
    }

    fn abort(&mut self) -> Result<(), String> {
        self.queued_prompts.clear();
        for pending in self.pending_inputs.drain().map(|(_, pending)| pending) {
            self.connection.respond(
                &pending.request,
                json!({"outcome": {"outcome": "cancelled"}}),
            )?;
        }
        self.connection
            .notify("session/cancel", json!({"sessionId": self.session_id}))
    }

    fn compact(&mut self) -> Result<(), String> {
        Err(format!(
            "{} does not expose ACP compaction",
            self.profile.name
        ))
    }

    fn rename(&mut self, name: &str) -> Result<(), String> {
        if self.profile.backend == "cursor-cli" {
            crate::modules::agents::adapter::cursor::rename_session(&self.session_id, name)
        } else {
            Err(format!(
                "{} does not expose ACP session naming",
                self.profile.name
            ))
        }
    }

    fn select_model(&mut self, _provider: &str, model: &str) -> Result<(), String> {
        let service_tier = self.config_ids.selected_service_tier.clone();
        let Some(config_id) = self.config_ids.model.clone() else {
            self.request_and_wait(
                "session/set_model",
                json!({"sessionId":self.session_id,"modelId":model}),
            )?;
            self.config_ids.selected_model = Some(model.into());
            return Ok(());
        };
        let selection = self.config_ids.selections.get(model).cloned();
        let base = selection
            .as_ref()
            .map_or(model, |selection| selection.model.as_str());
        self.request_and_wait(
            "session/set_config_option",
            json!({"sessionId": self.session_id, "configId": config_id, "value": base}),
        )?;
        if let Some(selection) = selection {
            for (config_id, value) in selection.parameters {
                self.request_and_wait(
                    "session/set_config_option",
                    json!({"sessionId":self.session_id,"configId":config_id,"value":value}),
                )?;
            }
        }
        if let Some(tier) = service_tier
            && self.config_ids.service_tiers.contains(&tier)
        {
            self.select_service_tier(&tier)?;
        }
        Ok(())
    }

    fn select_effort(&mut self, effort: &str) -> Result<(), String> {
        let config_id = self.config_ids.effort.clone().ok_or_else(|| {
            format!(
                "{} did not advertise an ACP effort option",
                self.profile.name
            )
        })?;
        self.request_and_wait(
            "session/set_config_option",
            json!({"sessionId": self.session_id, "configId": config_id, "value": effort}),
        )
    }

    fn select_service_tier(&mut self, tier: &str) -> Result<(), String> {
        if !self
            .config_ids
            .service_tiers
            .iter()
            .any(|option| option == tier)
        {
            return Err(format!(
                "Service tier is not available for this model: {tier}"
            ));
        }
        let config_id = self
            .config_ids
            .service_tier
            .clone()
            .ok_or("Agent did not advertise a service tier option")?;
        let value = super::configuration::CURSOR_SERVICE_TIERS
            .iter()
            .find(|(candidate, _)| *candidate == tier)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("Unknown service tier: {tier}"))?;
        self.request_and_wait(
            "session/set_config_option",
            json!({"sessionId":self.session_id,"configId":config_id,"value":value}),
        )
    }

    fn select_mode(&mut self, mode: &str) -> Result<(), String> {
        if let Some(config_id) = self.config_ids.mode.clone() {
            self.request_and_wait(
                "session/set_config_option",
                json!({"sessionId": self.session_id, "configId": config_id, "value": mode}),
            )
        } else {
            self.request_and_wait(
                "session/set_mode",
                json!({"sessionId": self.session_id, "modeId": mode}),
            )
        }
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.events.pop_front() {
            return Some(event);
        }
        if let Some(identity) = &self.caller_identity
            && let Some(message) = identity.try_recv()
        {
            self.peer_messages.push_back(message);
        }
        if self.current_prompt.is_none()
            && !self.peer_messages.is_empty()
            && self
                .caller_identity
                .as_ref()
                .is_none_or(|identity| identity.try_activate())
            && let Some(message) = self.peer_messages.pop_front()
        {
            let mode = WorkerSendMode::Prompt;
            return Some(match self.send_peer_message(&message, mode) {
                Ok(()) => WorkerEvent::Activity(WorkerActivity::PeerInputDelivered { message }),
                Err(error) => WorkerEvent::Failed(error),
            });
        }
        loop {
            let incoming = self.connection.poll()?;
            match incoming {
                Ok(AcpInbound::Response { id, result })
                    if let Some((message, images)) = self.pending_steers.remove(&id) =>
                {
                    if result.get("outcome").and_then(Value::as_str) == Some("promptRequired") {
                        self.queued_prompts
                            .push_front((WorkerSendMode::Steer, message, images));
                        continue;
                    }
                    return Some(WorkerEvent::Activity(WorkerActivity::InputDelivered {
                        mode: WorkerSendMode::Steer,
                        message,
                    }));
                }
                Ok(AcpInbound::Response { id, .. })
                    if self.current_prompt.as_ref() == Some(&id) =>
                {
                    self.current_prompt = None;
                    if let Some((mode, message, images)) = self.queued_prompts.pop_front() {
                        return Some(match self.start_prompt_request(&message, images) {
                            Ok(()) => WorkerEvent::Activity(WorkerActivity::InputDelivered {
                                mode,
                                message,
                            }),
                            Err(error) => WorkerEvent::Failed(error),
                        });
                    }
                    if let Some(identity) = &self.caller_identity {
                        identity.set_activity(WorkerActivityState::Idle);
                    }
                    return Some(WorkerEvent::Settled {
                        output: self.output.clone(),
                    });
                }
                Ok(AcpInbound::Response { .. }) => {}
                Ok(AcpInbound::Error { id, message }) => {
                    self.pending_steers.remove(&id);
                    if self.current_prompt.as_ref() == Some(&id) {
                        self.current_prompt = None;
                        if let Some(identity) = &self.caller_identity {
                            identity.set_activity(WorkerActivityState::Idle);
                        }
                    }
                    return Some(WorkerEvent::Failed(format!(
                        "{} ACP error: {message}",
                        self.profile.name
                    )));
                }
                Ok(AcpInbound::Notification { method, params }) => {
                    if method == "session/update" {
                        if let Some(event) = self.update(params) {
                            return Some(event);
                        }
                    } else if let Some(event) = self.cursor_notification(&method, &params) {
                        return Some(event);
                    } else {
                        log_bad_acp_message(
                            self.profile.name,
                            &method,
                            &params,
                            "unmapped notification",
                        );
                    }
                }
                Ok(AcpInbound::AgentRequest { id, method, params }) => {
                    if method == "session/request_permission"
                        && let Some(event) = self.permission_request(&id, &params)
                    {
                        return Some(event);
                    }
                    if let Some(event) = self.cursor_request(&id, &method, &params) {
                        return Some(event);
                    }
                    let reason = if method == "session/request_permission" {
                        "malformed permission request"
                    } else {
                        "unsupported agent request"
                    };
                    log_bad_acp_message(self.profile.name, &method, &params, reason);
                    if let Err(error) = self.reject_request(&id) {
                        return Some(WorkerEvent::Failed(error));
                    }
                }
                Err(error) => return Some(WorkerEvent::Failed(error)),
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
        if self.features.close
            && self
                .child
                .try_wait()
                .map_err(|error| format!("check {} ACP agent: {error}", self.profile.name))?
                .is_none()
        {
            let _ = self.request("session/close", json!({"sessionId": self.session_id}));
        }
        if self
            .child
            .try_wait()
            .map_err(|error| format!("check {} ACP agent: {error}", self.profile.name))?
            .is_none()
        {
            self.child
                .kill()
                .map_err(|error| format!("terminate {} ACP agent: {error}", self.profile.name))?;
        }
        self.child
            .wait()
            .map_err(|error| format!("reap {} ACP agent: {error}", self.profile.name))?;
        Ok(())
    }
}

impl Drop for AcpWorkerSession {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn log_bad_acp_message(profile: &str, method: &str, params: &Value, reason: &str) {
    zlog::warn!(
        "{profile} ACP message was not mapped correctly ({reason}): method={method} params={params}"
    );
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
