use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{BufRead as _, BufReader},
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::process::{PiRpcProcess, SessionLaunch};
use crate::{
    agents::extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
    agents::{
        AgentLaunchConfig, SessionActivityKind, SessionCommand, SessionEvent,
        SessionResponseErrorKind, WorkerContext, WorkerEvent, WorkerInput, WorkerInputResponse,
        WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
    },
};

#[derive(Clone)]
pub(crate) struct PiWorkerFactory {
    command: AgentLaunchConfig,
}

impl PiWorkerFactory {
    pub(crate) fn new(command: AgentLaunchConfig) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for PiWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("Pi worker provider and model must be supplied together".into());
        }
        if launch.ephemeral {
            return Err("Pi workers do not expose isolated ephemeral inference".into());
        }
        let mut command = self.command.clone();
        command.access_mode = launch.access_mode;
        command.app_proxy = launch.app_proxy.clone();
        let spawn = |start| {
            PiRpcProcess::spawn_worker(
                &command,
                &launch.project,
                start,
                launch.worker_id.clone(),
                launch.worker_name.clone(),
                launch
                    .parent_worker_id
                    .clone()
                    .map(|id| (id, launch.parent_session.clone())),
            )
        };
        let mut process = match &launch.context {
            WorkerContext::Fresh => spawn(SessionLaunch::New)?,
            WorkerContext::Resume { session_locator } => {
                let session = canonical_session(session_locator, "resume")?;
                spawn(SessionLaunch::Resume(&session))?
            }
            WorkerContext::Session { session_locator } => {
                let parent = canonical_session(&launch.parent_session, "parent")?;
                let source = canonical_session(session_locator, "source")?;
                if source != parent {
                    return Err(
                        "Pi workers cannot inherit context from a session other than their parent"
                            .into(),
                    );
                }
                if let Some(entry_id) = parent_before_worker_call(&source)? {
                    let mut process = spawn(SessionLaunch::Resume(&source))?;
                    process.request_and_wait(SessionCommand::ForkAt { entry_id })?;
                    process
                } else {
                    spawn(SessionLaunch::Fork(&source))?
                }
            }
        };
        process.set_worker_slot(launch.slot);
        process.request_and_wait(SessionCommand::ConfigureSteering)?;
        if let (Some(provider), Some(model_id)) = (launch.provider, launch.model) {
            process.request_and_wait(SessionCommand::SelectModel { provider, model_id })?;
        }
        if let Some(level) = launch.effort {
            process.request_and_wait(SessionCommand::SelectReasoning { level })?;
        }
        Ok(Box::new(PiWorkerSession {
            process,
            latest_output: String::new(),
            state_request: None,
            has_session_locator: false,
            settled: false,
            run_active: false,
            pending_inputs: HashMap::new(),
            prompt_requests: HashMap::new(),
            prompt_acks: VecDeque::new(),
            pending_session_events: VecDeque::new(),
            pending_worker_events: VecDeque::new(),
            terminal: false,
        }))
    }
}

struct PiWorkerSession {
    process: PiRpcProcess,
    latest_output: String,
    state_request: Option<String>,
    has_session_locator: bool,
    settled: bool,
    run_active: bool,
    pending_inputs: HashMap<String, InputKind>,
    prompt_requests: HashMap<String, PendingPrompt>,
    prompt_acks: VecDeque<(String, Result<(), String>)>,
    pending_session_events: VecDeque<SessionEvent>,
    pending_worker_events: VecDeque<WorkerEvent>,
    terminal: bool,
}

struct PendingPrompt {
    submission_id: String,
    mode: PromptMode,
    reports_ack: bool,
}

#[derive(Clone, Copy)]
enum InputKind {
    Value,
    Confirm,
}

impl WorkerSession for PiWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        self.send_prompt(None, message, mode, Vec::new())
    }

    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        self.send_prompt(None, message, mode, images)
    }

    fn submit_prompt(
        &mut self,
        id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        self.send_prompt(Some(id), message, mode, images)?;
        Ok(false)
    }

    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), String>)> {
        self.pump();
        self.prompt_acks.pop_front()
    }

    fn respond(&mut self, response: WorkerInputResponse) -> Result<(), String> {
        let kind = self
            .pending_inputs
            .remove(&response.id)
            .ok_or_else(|| format!("unknown Pi worker input: {}", response.id))?;
        let response = if response.cancel {
            ExtensionUiResponse::Cancelled {
                id: response.id,
                cancelled: true,
            }
        } else {
            let value = response
                .value
                .ok_or_else(|| "Pi worker response requires a value".to_owned())?;
            match kind {
                InputKind::Value => ExtensionUiResponse::Value {
                    id: response.id,
                    value,
                },
                InputKind::Confirm => ExtensionUiResponse::Confirmed {
                    id: response.id,
                    confirmed: matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "yes" | "true" | "allow" | "confirmed"
                    ),
                },
            }
        };
        self.process.send_extension_response(response)
    }

    fn abort(&mut self) -> Result<(), String> {
        self.process.send_request(SessionCommand::Abort)?;
        Ok(())
    }

    fn apply_steering(&mut self) -> Result<(), String> {
        self.process
            .send_request(SessionCommand::ApplySteering)
            .map(|_| ())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.pending_worker_events.pop_front() {
            return Some(event);
        }
        loop {
            let event = if let Some(event) = self.pending_session_events.pop_front() {
                event
            } else if self.terminal {
                return None;
            } else {
                self.process.try_next()?
            };
            match event {
                SessionEvent::Response(response)
                    if response
                        .id
                        .as_ref()
                        .is_some_and(|id| self.prompt_requests.contains_key(id)) =>
                {
                    self.route_prompt_response(response);
                    if let Some(event) = self.pending_worker_events.pop_front() {
                        return Some(event);
                    }
                }
                SessionEvent::Activity(event) => match event.kind() {
                    SessionActivityKind::AgentStarted => {
                        self.settled = false;
                        self.run_active = true;
                        self.latest_output.clear();
                        if let Err(error) = self.request_session_state() {
                            return Some(WorkerEvent::Failed(error));
                        }
                        return Some(WorkerEvent::Started);
                    }
                    SessionActivityKind::MessageEnded => {
                        if let Some(output) = final_assistant_text(event.value().get("message")) {
                            self.latest_output = output;
                        }
                    }
                    SessionActivityKind::AgentSettled => {
                        self.settled = true;
                        self.run_active = false;
                        if let Err(error) = self.request_session_state() {
                            return Some(WorkerEvent::Failed(error));
                        }
                        return Some(WorkerEvent::Settled {
                            output: self.latest_output.clone(),
                        });
                    }
                    _ => {}
                },
                SessionEvent::Interaction(request) => match worker_input(request) {
                    Ok(Some((input, kind))) => {
                        self.pending_inputs.insert(input.id.clone(), kind);
                        return Some(WorkerEvent::NeedsInput(input));
                    }
                    Ok(None) => {}
                    Err(error) => return Some(WorkerEvent::Failed(error)),
                },
                SessionEvent::Response(response)
                    if response.id.as_ref() == self.state_request.as_ref() =>
                {
                    self.state_request = None;
                    let Ok(crate::agents::SessionResponsePayload::LoadState(state)) =
                        response.result
                    else {
                        return Some(WorkerEvent::Failed(
                            "Pi worker expected session state".into(),
                        ));
                    };
                    if let Some(locator) = state.session_file {
                        self.has_session_locator = true;
                        return Some(WorkerEvent::SessionChanged { locator });
                    }
                    if self.settled {
                        return Some(WorkerEvent::Failed(
                            "Pi worker did not report a persistent session".into(),
                        ));
                    }
                }
                SessionEvent::Response(crate::agents::SessionResponse {
                    result: Err(error),
                    ..
                }) => {
                    return Some(WorkerEvent::RequestFailed {
                        operation: format!("Pi {:?}", error.operation),
                        error: error.to_string(),
                    });
                }
                SessionEvent::Failure(error) => {
                    self.terminal = true;
                    self.pending_session_events.clear();
                    return Some(WorkerEvent::Failed(error));
                }
                SessionEvent::Response(_) | SessionEvent::Stderr(_) => {}
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
        self.process.terminate()
    }
}

impl PiWorkerSession {
    fn send_prompt(
        &mut self,
        submission_id: Option<String>,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        let mode = prompt_mode(mode);
        let request_id = self.process.send_request(SessionCommand::Prompt {
            mode,
            message,
            images,
        })?;
        let reports_ack = submission_id.is_some();
        let submission_id =
            submission_id.unwrap_or_else(|| format!("pi-worker-input-{}", uuid::Uuid::new_v4()));
        self.prompt_requests.insert(
            request_id,
            PendingPrompt {
                submission_id,
                mode,
                reports_ack,
            },
        );
        Ok(())
    }

    fn pump(&mut self) {
        if self.terminal {
            return;
        }
        while let Some(event) = self.process.try_next() {
            match event {
                SessionEvent::Response(response)
                    if response
                        .id
                        .as_ref()
                        .is_some_and(|id| self.prompt_requests.contains_key(id)) =>
                {
                    self.route_prompt_response(response);
                }
                SessionEvent::Failure(error) => {
                    self.terminal = true;
                    self.pending_session_events.clear();
                    self.pending_worker_events
                        .push_back(WorkerEvent::Failed(error));
                    break;
                }
                event => {
                    self.pending_session_events.push_back(event);
                }
            }
        }
    }

    fn route_prompt_response(&mut self, response: crate::agents::SessionResponse) {
        let Some(request_id) = response.id.as_ref() else {
            return;
        };
        let Some(prompt) = self.prompt_requests.remove(request_id) else {
            return;
        };
        match response.result {
            Ok(_) if prompt.reports_ack => {
                self.prompt_acks.push_back((prompt.submission_id, Ok(())))
            }
            Ok(_) => {}
            Err(error) if error.kind == SessionResponseErrorKind::DeliveryUnknown => {
                self.pending_worker_events
                    .push_back(WorkerEvent::PromptDeliveryUnknown {
                        submission_id: prompt.submission_id,
                        error: error.to_string(),
                    });
            }
            Err(error) if prompt.reports_ack => self
                .prompt_acks
                .push_back((prompt.submission_id, Err(error.to_string()))),
            Err(error) => {
                if prompt.mode == PromptMode::Normal && !self.run_active {
                    self.pending_worker_events
                        .push_back(WorkerEvent::Failed(error.to_string()));
                } else {
                    self.pending_worker_events
                        .push_back(WorkerEvent::RequestFailed {
                            operation: format!("Pi {:?}", error.operation),
                            error: error.to_string(),
                        });
                }
            }
        }
    }

    fn request_session_state(&mut self) -> Result<(), String> {
        if !self.has_session_locator && self.state_request.is_none() {
            self.state_request = Some(self.process.send_request(SessionCommand::LoadState)?);
        }
        Ok(())
    }
}

const fn prompt_mode(mode: WorkerSendMode) -> PromptMode {
    match mode {
        WorkerSendMode::Prompt => PromptMode::Normal,
        WorkerSendMode::Queue => PromptMode::FollowUp,
        WorkerSendMode::Steer => PromptMode::Steer,
    }
}

fn canonical_session(locator: &str, role: &str) -> Result<PathBuf, String> {
    let session = Path::new(locator)
        .canonicalize()
        .map_err(|error| format!("resolve Pi worker {role} session: {error}"))?;
    if !session.is_file() {
        return Err(format!(
            "Pi worker {role} session is not a file: {}",
            session.display()
        ));
    }
    Ok(session)
}

fn parent_before_worker_call(path: &Path) -> Result<Option<String>, String> {
    let file = File::open(path)
        .map_err(|error| format!("open Pi worker source session {}: {error}", path.display()))?;
    let mut leaf = None;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| format!("read Pi worker source session: {error}"))?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if entry["id"].is_string() {
            leaf = Some(entry);
        }
    }
    let Some(leaf) = leaf else {
        return Ok(None);
    };
    let starts_worker = leaf["type"].as_str() == Some("message")
        && leaf["message"]["role"].as_str() == Some("assistant")
        && leaf["message"]["content"]
            .as_array()
            .is_some_and(|content| {
                content.iter().any(|part| {
                    part["type"].as_str() == Some("toolCall")
                        && part["name"].as_str() == Some("worker_start")
                })
            });
    if !starts_worker {
        return Ok(None);
    }
    Ok(leaf["parentId"].as_str().map(str::to_owned))
}

fn worker_input(request: ExtensionUiRequest) -> Result<Option<(WorkerInput, InputKind)>, String> {
    match request {
        ExtensionUiRequest::Select {
            id, title, options, ..
        } => Ok(Some((
            WorkerInput {
                id,
                prompt: title,
                options,
                secret: false,
            },
            InputKind::Value,
        ))),
        ExtensionUiRequest::Confirm {
            id, title, message, ..
        } => Ok(Some((
            WorkerInput {
                id,
                prompt: format!("{title}\n{message}"),
                options: vec!["Yes".into(), "No".into()],
                secret: false,
            },
            InputKind::Confirm,
        ))),
        ExtensionUiRequest::Input {
            id,
            title,
            placeholder,
            ..
        }
        | ExtensionUiRequest::Editor {
            id,
            title,
            prefill: placeholder,
        } => Ok(Some((
            WorkerInput {
                id,
                prompt: placeholder.map_or(title.clone(), |placeholder| {
                    format!("{title}\n{placeholder}")
                }),
                options: Vec::new(),
                secret: false,
            },
            InputKind::Value,
        ))),
        ExtensionUiRequest::Unknown { method, .. } => {
            Err(format!("unsupported Pi worker interaction: {method}"))
        }
        _ => Ok(None),
    }
}

fn final_assistant_text(message: Option<&Value>) -> Option<String> {
    let message = message?;
    if message["role"].as_str() != Some("assistant") {
        return None;
    }
    if let Some(text) = message["content"].as_str() {
        return Some(text.to_owned());
    }
    Some(
        message["content"]
            .as_array()?
            .iter()
            .filter_map(|part| {
                (part["type"].as_str() == Some("text"))
                    .then(|| part["text"].as_str())
                    .flatten()
            })
            .collect::<String>(),
    )
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
