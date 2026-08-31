use std::{collections::VecDeque, path::PathBuf};

use serde_json::{Value, json};

use crate::agents::{
    SessionCommand, SessionEvent, SessionResponse, SessionTransport, WorkerEvent, WorkerInput,
    WorkerInputResponse, WorkerSendMode, WorkerSession,
    extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
};

#[derive(Default)]
pub(super) struct MainSessionMetadata {
    pub models: Vec<Value>,
    pub efforts: Vec<String>,
    pub commands: Vec<Value>,
    pub modes: Vec<Value>,
}

pub(super) struct WorkerSessionTransport {
    harness: String,
    locator: String,
    path: PathBuf,
    worker: Box<dyn WorkerSession>,
    pending: VecDeque<SessionEvent>,
    next_id: u64,
    running: bool,
    rich_activity: bool,
    model: Option<(String, String)>,
    effort: String,
    metadata: MainSessionMetadata,
    selected_mode: Option<String>,
}

impl WorkerSessionTransport {
    pub(super) fn new(
        harness: &str,
        locator: String,
        worker: Box<dyn WorkerSession>,
        metadata: MainSessionMetadata,
    ) -> Result<Self, String> {
        let path = external_session_path(harness, &locator)?;
        Ok(Self {
            harness: harness.into(),
            locator,
            path,
            worker,
            pending: VecDeque::new(),
            next_id: 0,
            running: false,
            rich_activity: false,
            model: None,
            effort: metadata
                .efforts
                .first()
                .cloned()
                .unwrap_or_else(|| "off".into()),
            metadata,
            selected_mode: None,
        })
    }

    fn response(&mut self, id: String, command: &str, data: Value) {
        self.pending
            .push_back(SessionEvent::Response(SessionResponse {
                id: Some(id),
                command: command.into(),
                success: true,
                data,
                error: None,
            }));
    }

    fn state(&self) -> Value {
        let model = self.model.as_ref().map(|(provider, id)| {
            json!({
                "id": id,
                "name": id,
                "provider": provider,
                "contextWindow": 0,
                "reasoning": true,
            })
        });
        json!({
            "model": model,
            "thinkingLevel": self.effort,
            "isStreaming": self.running,
            "isCompacting": false,
            "sessionFile": self.path.to_string_lossy(),
            "sessionId": self.locator,
            "sessionName": null,
            "autoCompactionEnabled": true,
            "messageCount": 0,
            "pendingMessageCount": 0,
        })
    }

    fn worker_event(&mut self, event: WorkerEvent) -> SessionEvent {
        match event {
            WorkerEvent::Started => {
                self.running = true;
                SessionEvent::Activity(json!({"type": "agent_start"}))
            }
            WorkerEvent::Settled { output } => {
                self.running = false;
                if !self.rich_activity {
                    self.pending.push_back(SessionEvent::Activity(json!({
                        "type": "message_start",
                        "message": {"role": "assistant", "content": []}
                    })));
                    self.pending.push_back(SessionEvent::Activity(json!({
                        "type": "message_update",
                        "assistantMessageEvent": {
                            "type": "text_delta",
                            "contentIndex": 0,
                            "delta": output,
                        }
                    })));
                    self.pending.push_back(SessionEvent::Activity(json!({
                        "type": "message_end",
                        "message": {"role": "assistant"}
                    })));
                }
                self.rich_activity = false;
                SessionEvent::Activity(json!({"type": "agent_settled"}))
            }
            WorkerEvent::SessionChanged { locator } => {
                self.locator = locator;
                SessionEvent::Activity(json!({"type": "session_info_changed"}))
            }
            WorkerEvent::NeedsInput(input) => SessionEvent::Interaction(interaction(input)),
            WorkerEvent::Activity(activity) => {
                self.rich_activity = true;
                SessionEvent::Activity(activity)
            }
            WorkerEvent::Failed(error) => SessionEvent::Failure(error),
        }
    }
}

impl SessionTransport for WorkerSessionTransport {
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        self.next_id = self.next_id.saturating_add(1);
        let id = format!("{}-{}", self.harness, self.next_id);
        match command {
            SessionCommand::ConfigureSteering => self.response(id.clone(), "set_steering_mode", json!({})),
            SessionCommand::LoadState => {
                self.response(id.clone(), "get_state", self.state());
            }
            SessionCommand::LoadHistory => self.response(
                id.clone(),
                "get_entries",
                json!({"entries": [], "preserve": true}),
            ),
            SessionCommand::LoadUsage => self.response(
                id.clone(),
                "get_session_stats",
                json!({"tokens": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}),
            ),
            SessionCommand::ListModels => self.response(
                id.clone(),
                "get_available_models",
                json!({"models": self.metadata.models.clone()}),
            ),
            SessionCommand::ListReasoningLevels => self.response(
                id.clone(),
                "get_available_thinking_levels",
                json!({"levels": self.metadata.efforts.clone()}),
            ),
            SessionCommand::ListModes => self.response(
                id.clone(),
                "get_modes",
                json!({"modes": self.metadata.modes.clone(), "selected": self.selected_mode}),
            ),
            SessionCommand::ListCommands => self.response(
                id.clone(),
                "get_commands",
                json!({"commands": self.metadata.commands.clone()}),
            ),
            SessionCommand::Prompt {
                mode,
                message,
                images,
            } => {
                let worker_mode = match mode {
                    PromptMode::Normal => WorkerSendMode::Prompt,
                    PromptMode::Steer => WorkerSendMode::Steer,
                    PromptMode::FollowUp => WorkerSendMode::Queue,
                };
                self.worker
                    .send_with_images(message, worker_mode, images)?;
                let command = match mode {
                    PromptMode::Normal => "prompt",
                    PromptMode::Steer => "steer",
                    PromptMode::FollowUp => "follow_up",
                };
                self.response(id.clone(), command, json!({}));
            }
            SessionCommand::Abort => {
                self.worker.abort()?;
                self.response(id.clone(), "abort", json!({}));
            }
            SessionCommand::SelectModel { provider, model_id } => {
                self.worker.select_model(&provider, &model_id)?;
                self.model = Some((provider.clone(), model_id.clone()));
                self.response(
                    id.clone(),
                    "set_model",
                    json!({"id": model_id, "name": model_id, "provider": provider, "contextWindow": 0, "reasoning": true}),
                );
            }
            SessionCommand::SelectReasoning { level } => {
                self.worker.select_effort(&level)?;
                self.effort = level;
                self.response(id.clone(), "set_thinking_level", json!({}));
            }
            SessionCommand::SelectMode { mode } => {
                self.worker.select_mode(&mode)?;
                self.selected_mode = Some(mode);
                self.response(id.clone(), "set_mode", json!({}));
            }
            SessionCommand::Compact { .. } => {
                self.worker.compact()?;
                self.response(id.clone(), "compact", json!({}));
            }
            SessionCommand::Rename { name } => {
                self.worker.rename(&name)?;
                self.response(id.clone(), "set_session_name", json!({}));
            }
            SessionCommand::ExportHtml { .. } | SessionCommand::ForkAt { .. } => {
                return Err(format!(
                    "{} does not expose this command through its main-session bridge yet",
                    self.harness
                ));
            }
        }
        Ok(id)
    }

    fn respond(&mut self, response: ExtensionUiResponse) -> Result<(), String> {
        let response = match response {
            ExtensionUiResponse::Value { id, value } => WorkerInputResponse {
                id,
                value: Some(value),
                cancel: false,
            },
            ExtensionUiResponse::Confirmed { id, confirmed } => WorkerInputResponse {
                id,
                value: Some(if confirmed { "allow" } else { "decline" }.into()),
                cancel: false,
            },
            ExtensionUiResponse::Cancelled { id, .. } => WorkerInputResponse {
                id,
                value: None,
                cancel: true,
            },
        };
        self.worker.respond(response)
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending
            .pop_front()
            .or_else(|| self.worker.poll().map(|event| self.worker_event(event)))
    }

    fn close(&mut self) -> Result<(), String> {
        self.worker.close()
    }
}

fn interaction(input: WorkerInput) -> ExtensionUiRequest {
    if input.options.is_empty() {
        ExtensionUiRequest::Input {
            id: input.id,
            title: input.prompt,
            placeholder: None,
            timeout: None,
        }
    } else if input.options.len() == 2 {
        ExtensionUiRequest::Confirm {
            id: input.id,
            title: input.prompt.clone(),
            message: input.prompt,
            timeout: None,
        }
    } else {
        ExtensionUiRequest::Select {
            id: input.id,
            title: input.prompt,
            options: input.options,
            timeout: None,
        }
    }
}

pub(in crate::modules::agents::adapter) fn external_session_path(
    harness: &str,
    locator: &str,
) -> Result<PathBuf, String> {
    let encoded = url::form_urlencoded::byte_serialize(locator.as_bytes()).collect::<String>();
    crate::app::paths::data_dir()
        .map(|root| root.join("session-locators").join(harness).join(encoded))
}

pub(in crate::modules::agents::adapter) fn external_session_locator(
    harness: &str,
    path: &std::path::Path,
) -> Option<String> {
    (path.parent()?.file_name()?.to_str()? == harness)
        .then(|| percent_decode(path.file_name()?.to_str()?))
        .flatten()
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex(bytes.get(index + 1).copied()?)?;
            let low = hex(bytes.get(index + 2).copied()?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
