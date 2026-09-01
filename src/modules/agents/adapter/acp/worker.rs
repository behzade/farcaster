use std::{
    collections::{HashMap, VecDeque},
    io::{BufReader, Write as _},
    process::{Child, ChildStdin, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use super::{
    AcpProfile,
    connection::{AcpConnection, read_message},
    translate::{
        ConfigIds, content_text, find_permission_option, is_acceptance, metadata_from_options,
        metadata_from_session, normalize_content, normalize_tool_name, tool_content, usage_update,
    },
    wire::{AcpInbound, AcpRequestId, encode_notification, encode_request, encode_response},
};
use crate::{
    agents::{
        AgentLaunchConfig, HarnessAccessMode, WorkerActivity, WorkerActivityState, WorkerEvent,
        WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession,
        WorkerSessionFactory,
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
        let caller_identity = crate::modules::agents::core::CallerRegistry::shared().issue_as(
            &launch.project,
            crate::modules::agents::core::CallerProfile {
                backend: self.profile.backend.into(),
                provider: launch.provider.clone(),
                model: launch.model.clone(),
                effort: launch.effort.clone(),
            },
            None,
            launch.worker_id.clone(),
        );
        if matches!(launch.context, crate::agents::WorkerContext::Session { .. }) {
            return Err(format!(
                "{} does not advertise ACP session fork for inherited workers",
                self.profile.name
            ));
        }
        let (mut session, _) = spawn_session(
            &self.command,
            &self.profile,
            &launch.project,
            None,
            Some(caller_identity.token()),
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
        if let Some(effort) = launch.effort.as_deref() {
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
) -> Result<
    (
        Box<dyn WorkerSession>,
        String,
        super::super::main_session::MainSessionMetadata,
    ),
    String,
> {
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
    let (session, metadata) = spawn_session(
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
    ),
    String,
> {
    let mut prepared = command.command(project)?;
    configure_command(&mut prepared, profile, command.access_mode);
    let mut child = prepared
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start {} ACP agent: {error}", profile.name))?;
    child_stderr::capture(&mut child, "acp-agent")?;
    let setup = setup_connection(&mut child, profile, project, resume, caller_token);
    let (mut reader, writer, queued, next_id, session_id, metadata, config_ids) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let (sender, incoming) = mpsc::channel();
    let reader_name = session_id.clone();
    thread::Builder::new()
        .name(format!("acp-session-{reader_name}"))
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
        .map_err(|error| format!("read {} ACP events: {error}", profile.name))?;
    Ok((
        AcpWorkerSession {
            profile: profile.clone(),
            child,
            writer,
            incoming,
            deferred: VecDeque::new(),
            session_id,
            next_id,
            current_prompt: None,
            output: String::new(),
            thought_started: false,
            pending_inputs: HashMap::new(),
            tool_states: HashMap::new(),
            peer_messages: VecDeque::new(),
            events: VecDeque::new(),
            config_ids,
            caller_identity: None,
        },
        metadata,
    ))
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

type AcpSetup = (
    BufReader<std::process::ChildStdout>,
    ChildStdin,
    VecDeque<AcpInbound>,
    i64,
    String,
    super::super::main_session::MainSessionMetadata,
    ConfigIds,
);

fn setup_connection(
    child: &mut Child,
    profile: &AcpProfile,
    project: &std::path::Path,
    resume: Option<&str>,
    caller_token: Option<&str>,
) -> Result<AcpSetup, String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("{} ACP stdin must be piped", profile.name))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{} ACP stdout must be piped", profile.name))?;
    let mut connection = AcpConnection::new(BufReader::new(stdout), stdin);
    let initialized = connection.initialize(profile)?;
    if initialized.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
        return Err(format!(
            "{} ACP agent did not negotiate protocol version 1",
            profile.name
        ));
    }
    let params = json!({
        "cwd": project.to_string_lossy(),
        "mcpServers": acp_mcp_servers(caller_token),
    });
    let id = if let Some(session_id) = resume {
        connection.send_request(
            "session/load",
            merge(params, "sessionId", Value::String(session_id.into())),
        )?
    } else {
        connection.send_request("session/new", params)?
    };
    let response = connection.wait_response(&id)?;
    let session_id = response
        .get("sessionId")
        .and_then(Value::as_str)
        .or(resume)
        .ok_or_else(|| format!("{} ACP agent did not provide a session id", profile.name))?
        .to_owned();
    let (metadata, config_ids) = metadata_from_session(profile, &response);
    if resume.is_some() {
        let _ = connection.drain_queued();
    }
    let (reader, writer, queued, next_id) = connection.into_parts();
    Ok((
        reader, writer, queued, next_id, session_id, metadata, config_ids,
    ))
}

fn merge(mut object: Value, key: &str, value: Value) -> Value {
    object[key] = value;
    object
}

pub(in crate::modules::agents::adapter) fn configure_command(
    command: &mut std::process::Command,
    profile: &AcpProfile,
    access_mode: HarnessAccessMode,
) {
    if access_mode == HarnessAccessMode::Full
        && let Some(argument) = profile.force_argument
    {
        command.arg(argument);
    }
    command.args(profile.arguments);
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

struct PendingInput {
    request: AcpRequestId,
    option_ids: HashMap<String, String>,
    allow_option: Option<String>,
    reject_option: Option<String>,
}

#[derive(Clone, Default)]
struct ToolState {
    name: String,
    started: bool,
    finished: bool,
}

struct AcpWorkerSession {
    profile: AcpProfile,
    child: Child,
    writer: ChildStdin,
    incoming: mpsc::Receiver<Result<AcpInbound, String>>,
    deferred: VecDeque<Result<AcpInbound, String>>,
    session_id: String,
    next_id: i64,
    current_prompt: Option<AcpRequestId>,
    output: String,
    thought_started: bool,
    pending_inputs: HashMap<String, PendingInput>,
    tool_states: HashMap<String, ToolState>,
    peer_messages: VecDeque<String>,
    events: VecDeque<WorkerEvent>,
    config_ids: ConfigIds,
    caller_identity: Option<crate::modules::agents::core::CallerIdentity>,
}

impl AcpWorkerSession {
    fn with_identity(mut self, identity: crate::modules::agents::core::CallerIdentity) -> Self {
        self.caller_identity = Some(identity);
        self
    }

    fn request(&mut self, method: &str, params: Value) -> Result<AcpRequestId, String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| "ACP request id overflow".to_owned())?;
        let id = AcpRequestId::Number(self.next_id);
        self.writer
            .write_all(&encode_request(&id, method, params)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("write {} ACP request: {error}", self.profile.name))?;
        Ok(id)
    }

    fn request_and_wait(&mut self, method: &str, params: Value) -> Result<(), String> {
        let expected = self.request(method, params)?;
        loop {
            let message = self
                .incoming
                .recv_timeout(Duration::from_secs(15))
                .map_err(|error| format!("wait for {} ACP response: {error}", self.profile.name))?;
            match message {
                Ok(AcpInbound::Response { id, .. }) if id == expected => return Ok(()),
                Ok(AcpInbound::Error { id, code, message }) if id == expected => {
                    return Err(format!("{} ACP error {code}: {message}", self.profile.name));
                }
                Err(error) => return Err(error),
                other => self.deferred.push_back(other),
            }
        }
    }

    fn update(&mut self, params: Value) -> Option<WorkerEvent> {
        if params.get("sessionId").and_then(Value::as_str) != Some(&self.session_id) {
            return None;
        }
        let update = params.get("update")?;
        match update.get("sessionUpdate").and_then(Value::as_str)? {
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
            "config_option_update" => {
                if let Some(options) = update.get("configOptions").and_then(Value::as_array) {
                    let (_, ids) = metadata_from_options(&self.profile, options);
                    self.config_ids = ids;
                }
                None
            }
            _ => None,
        }
    }

    fn tool_update(&mut self, update: &Value) -> Option<WorkerEvent> {
        let id = update.get("toolCallId")?.as_str()?.to_owned();
        let state = self.tool_states.entry(id.clone()).or_default();
        if let Some(title) = update.get("title").and_then(Value::as_str) {
            state.name = normalize_tool_name(update, title);
        }
        if !state.started {
            state.started = true;
            let name = if state.name.is_empty() {
                "tool".into()
            } else {
                state.name.clone()
            };
            let status = update.get("status").and_then(Value::as_str);
            if matches!(status, Some("completed" | "failed")) {
                state.finished = true;
                self.events
                    .push_back(WorkerEvent::Activity(WorkerActivity::ToolFinished {
                        id: id.clone(),
                        result: tool_content(update),
                        is_error: status == Some("failed"),
                    }));
            }
            return Some(WorkerEvent::Activity(WorkerActivity::ToolStarted {
                id,
                name,
                args: update.get("rawInput").cloned().unwrap_or_else(|| json!({})),
            }));
        }
        let status = update.get("status").and_then(Value::as_str);
        if matches!(status, Some("completed" | "failed")) && !state.finished {
            state.finished = true;
            return Some(WorkerEvent::Activity(WorkerActivity::ToolFinished {
                id,
                result: tool_content(update),
                is_error: status == Some("failed"),
            }));
        }
        update
            .get("content")
            .or_else(|| update.get("rawOutput"))
            .map(|content| {
                WorkerEvent::Activity(WorkerActivity::ToolUpdated {
                    id,
                    content: normalize_content(content),
                })
            })
    }

    fn permission_request(&mut self, id: AcpRequestId, params: Value) -> Option<WorkerEvent> {
        if params.get("sessionId").and_then(Value::as_str) != Some(&self.session_id) {
            return None;
        }
        let options = params.get("options")?.as_array()?;
        let input_id = request_id_string(&id);
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
                request: id,
                option_ids,
                allow_option,
                reject_option,
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
        if mode != WorkerSendMode::Prompt {
            return Err("ACP v1 does not support steering or queued prompts".into());
        }
        if self.current_prompt.is_some() {
            return Err(format!(
                "{} ACP session is already working",
                self.profile.name
            ));
        }
        let mut prompt = vec![json!({"type": "text", "text": message})];
        prompt.extend(images.into_iter().map(
            |image| json!({"type": "image", "mimeType": image.mime_type, "data": image.data}),
        ));
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
        self.events.push_back(WorkerEvent::Started);
        Ok(())
    }

    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        let pending = self
            .pending_inputs
            .remove(&response.id)
            .ok_or_else(|| format!("unknown ACP interaction: {}", response.id))?;
        let result = if response.cancel {
            json!({"outcome": {"outcome": "cancelled"}})
        } else {
            let value = response.value.as_deref().unwrap_or_default();
            let option_id = pending.option_ids.get(value).cloned().or_else(|| {
                if is_acceptance(value) {
                    pending.allow_option
                } else {
                    pending.reject_option
                }
            });
            let option_id = option_id
                .ok_or_else(|| "ACP permission response has no matching option".to_owned())?;
            json!({"outcome": {"outcome": "selected", "optionId": option_id}})
        };
        self.writer
            .write_all(&encode_response(&pending.request, result)?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("answer {} ACP request: {error}", self.profile.name))
    }

    fn abort(&mut self) -> Result<(), String> {
        for pending in self.pending_inputs.drain().map(|(_, pending)| pending) {
            self.writer
                .write_all(&encode_response(
                    &pending.request,
                    json!({"outcome": {"outcome": "cancelled"}}),
                )?)
                .map_err(|error| format!("cancel {} ACP permission: {error}", self.profile.name))?;
        }
        self.writer
            .write_all(&encode_notification(
                "session/cancel",
                json!({"sessionId": self.session_id}),
            )?)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("cancel {} ACP prompt: {error}", self.profile.name))
    }

    fn select_model(&mut self, _provider: &str, model: &str) -> Result<(), String> {
        let config_id = self.config_ids.model.clone().ok_or_else(|| {
            format!(
                "{} did not advertise an ACP model option",
                self.profile.name
            )
        })?;
        self.request_and_wait(
            "session/set_config_option",
            json!({"sessionId": self.session_id, "configId": config_id, "value": model}),
        )
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
            self.peer_messages.push_back(message.prompt());
        }
        if self.current_prompt.is_none()
            && let Some(message) = self.peer_messages.pop_front()
        {
            return Some(match self.send(message, WorkerSendMode::Prompt) {
                Ok(()) => self.events.pop_front().unwrap_or(WorkerEvent::Started),
                Err(error) => WorkerEvent::Failed(error),
            });
        }
        loop {
            let incoming = self
                .deferred
                .pop_front()
                .or_else(|| self.incoming.try_recv().ok())?;
            match incoming {
                Ok(AcpInbound::Response { id, .. })
                    if self.current_prompt.as_ref() == Some(&id) =>
                {
                    self.current_prompt = None;
                    if let Some(identity) = &self.caller_identity {
                        identity.set_activity(WorkerActivityState::Idle);
                    }
                    return Some(WorkerEvent::Settled {
                        output: self.output.clone(),
                    });
                }
                Ok(AcpInbound::Response { .. }) => {}
                Ok(AcpInbound::Error { id, code, message }) => {
                    if self.current_prompt.as_ref() == Some(&id) {
                        self.current_prompt = None;
                        if let Some(identity) = &self.caller_identity {
                            identity.set_activity(WorkerActivityState::Idle);
                        }
                    }
                    return Some(WorkerEvent::Failed(format!(
                        "{} ACP error {code}: {message}",
                        self.profile.name
                    )));
                }
                Ok(AcpInbound::Notification { method, params }) => {
                    if method == "session/update"
                        && let Some(event) = self.update(params)
                    {
                        return Some(event);
                    }
                }
                Ok(AcpInbound::AgentRequest { id, method, params }) => {
                    if method == "session/request_permission" {
                        if let Some(event) = self.permission_request(id, params) {
                            return Some(event);
                        }
                    } else {
                        let response = json!({"outcome": {"outcome": "cancelled"}});
                        let encoded = match encode_response(&id, response) {
                            Ok(encoded) => encoded,
                            Err(error) => return Some(WorkerEvent::Failed(error)),
                        };
                        if let Err(error) = self
                            .writer
                            .write_all(&encoded)
                            .and_then(|()| self.writer.flush())
                        {
                            return Some(WorkerEvent::Failed(format!(
                                "reject unsupported {} ACP request: {error}",
                                self.profile.name
                            )));
                        }
                    }
                }
                Err(error) => return Some(WorkerEvent::Failed(error)),
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
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

fn request_id_string(id: &AcpRequestId) -> String {
    match id {
        AcpRequestId::Number(value) => value.to_string(),
        AcpRequestId::String(value) => value.clone(),
        AcpRequestId::Null => "null".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: AcpProfile = AcpProfile {
        backend: "test-acp",
        name: "Test ACP",
        command: "test-acp",
        path_environment: "FARCASTER_TEST_ACP_PATH",
        arguments: &["acp"],
        auth_method: None,
        force_argument: Some("--force"),
    };

    #[test]
    fn full_access_uses_the_profile_escape_hatch() {
        let mut command = std::process::Command::new("agent");
        configure_command(&mut command, &PROFILE, HarnessAccessMode::Full);
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["--force", "acp"]
        );
    }
}
