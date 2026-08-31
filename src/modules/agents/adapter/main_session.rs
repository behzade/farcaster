use std::{collections::VecDeque, path::PathBuf};

use serde_json::{Value, json};

use crate::agents::{
    SessionCommand, SessionEvent, SessionResponse, SessionTransport, WorkerEvent, WorkerInput,
    WorkerInputResponse, WorkerSendMode, WorkerSession,
    extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode},
};

pub(super) struct WorkerSessionTransport {
    harness: String,
    locator: String,
    path: PathBuf,
    worker: Box<dyn WorkerSession>,
    pending: VecDeque<SessionEvent>,
    next_id: u64,
    running: bool,
    model: Option<(String, String)>,
    effort: String,
}

impl WorkerSessionTransport {
    pub(super) fn new(
        harness: &str,
        locator: String,
        worker: Box<dyn WorkerSession>,
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
            model: None,
            effort: "off".into(),
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
                SessionEvent::Activity(json!({"type": "agent_settled"}))
            }
            WorkerEvent::SessionChanged { locator } => {
                self.locator = locator;
                SessionEvent::Activity(json!({"type": "session_info_changed"}))
            }
            WorkerEvent::NeedsInput(input) => SessionEvent::Interaction(interaction(input)),
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
                json!({"models": []}),
            ),
            SessionCommand::ListReasoningLevels => self.response(
                id.clone(),
                "get_available_thinking_levels",
                json!({"levels": ["off", "minimal", "low", "medium", "high", "xhigh"]}),
            ),
            SessionCommand::ListCommands => {
                self.response(id.clone(), "get_commands", json!({"commands": []}));
            }
            SessionCommand::Prompt {
                mode,
                message,
                images,
            } => {
                if !images.is_empty() {
                    return Err(format!(
                        "{} main-session image delivery is not connected yet",
                        self.harness
                    ));
                }
                let worker_mode = match mode {
                    PromptMode::Normal => WorkerSendMode::Prompt,
                    PromptMode::Steer => WorkerSendMode::Steer,
                    PromptMode::FollowUp => WorkerSendMode::Queue,
                };
                self.worker.send(message, worker_mode)?;
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
                self.model = Some((provider.clone(), model_id.clone()));
                self.response(
                    id.clone(),
                    "set_model",
                    json!({"id": model_id, "name": model_id, "provider": provider, "contextWindow": 0, "reasoning": true}),
                );
            }
            SessionCommand::SelectReasoning { level } => {
                self.effort = level;
                self.response(id.clone(), "set_thinking_level", json!({}));
            }
            SessionCommand::Compact { .. }
            | SessionCommand::ExportHtml { .. }
            | SessionCommand::Rename { .. }
            | SessionCommand::ForkAt { .. } => {
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
    if input.options.len() == 2 {
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

fn external_session_path(harness: &str, locator: &str) -> Result<PathBuf, String> {
    let encoded = url::form_urlencoded::byte_serialize(locator.as_bytes()).collect::<String>();
    crate::app_paths::data_dir()
        .map(|root| root.join("session-locators").join(harness).join(encoded))
}
