use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead as _, BufReader},
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::process::PiRpcProcess;
use crate::{
    agents::extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode, SessionState},
    agents::{
        AgentLaunchConfig, SessionActivityKind, SessionCommand, SessionEvent, WorkerContext,
        WorkerEvent, WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode, WorkerSession,
        WorkerSessionFactory,
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
        let mut process = match &launch.context {
            WorkerContext::Fresh => PiRpcProcess::spawn_worker(
                &self.command,
                &launch.project,
                launch.worker_id.clone(),
            )?,
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
                    let mut process =
                        PiRpcProcess::spawn(&self.command, &launch.project, Some(&source))?;
                    process.request_and_wait(SessionCommand::ForkAt { entry_id })?;
                    process
                } else {
                    PiRpcProcess::spawn_fork(&self.command, &launch.project, &source)?
                }
            }
        };
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
            pending_inputs: HashMap::new(),
        }))
    }
}

struct PiWorkerSession {
    process: PiRpcProcess,
    latest_output: String,
    state_request: Option<String>,
    has_session_locator: bool,
    settled: bool,
    pending_inputs: HashMap<String, InputKind>,
}

#[derive(Clone, Copy)]
enum InputKind {
    Value,
    Confirm,
}

impl WorkerSession for PiWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        let mode = match mode {
            WorkerSendMode::Prompt => PromptMode::Normal,
            WorkerSendMode::Queue => PromptMode::FollowUp,
            WorkerSendMode::Steer => PromptMode::Steer,
        };
        self.process.send_request(SessionCommand::Prompt {
            mode,
            message,
            images: Vec::new(),
        })?;
        Ok(())
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

    fn poll(&mut self) -> Option<WorkerEvent> {
        loop {
            match self.process.try_next()? {
                SessionEvent::Activity(event) => match event.kind() {
                    SessionActivityKind::AgentStarted => {
                        self.settled = false;
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
                SessionEvent::Response(response) if !response.success => {
                    return Some(WorkerEvent::Failed(
                        response
                            .error
                            .unwrap_or_else(|| format!("Pi rejected {:?}", response.operation)),
                    ));
                }
                SessionEvent::Response(response)
                    if response.id.as_ref() == self.state_request.as_ref() =>
                {
                    self.state_request = None;
                    let state = match serde_json::from_value::<SessionState>(response.data) {
                        Ok(state) => state,
                        Err(error) => {
                            return Some(WorkerEvent::Failed(format!(
                                "invalid Pi worker session state: {error}"
                            )));
                        }
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
                SessionEvent::Failure(error) => return Some(WorkerEvent::Failed(error)),
                SessionEvent::Response(_) | SessionEvent::Stderr(_) => {}
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
        self.process.terminate()
    }
}

impl PiWorkerSession {
    fn request_session_state(&mut self) -> Result<(), String> {
        if !self.has_session_locator && self.state_request.is_none() {
            self.state_request = Some(self.process.send_request(SessionCommand::LoadState)?);
        }
        Ok(())
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn worker_output_uses_only_final_assistant_text() {
        let message = json!({
            "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": "private"},
                {"type": "text", "text": "first"},
                {"type": "text", "text": " second"}
            ]
        });
        assert_eq!(
            final_assistant_text(Some(&message)).as_deref(),
            Some("first second")
        );
        assert_eq!(
            final_assistant_text(Some(&json!({"role":"user","content":"no"}))),
            None
        );
    }

    #[test]
    fn worker_fork_omits_the_active_worker_start_call() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::NamedTempFile::new()?;
        std::fs::write(
            temp.path(),
            concat!(
                "{\"type\":\"session\",\"version\":3,\"id\":\"session-1\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"cwd\":\"/project\"}\n",
                "{\"type\":\"message\",\"id\":\"user-1\",\"parentId\":null,\"message\":{\"role\":\"user\",\"content\":\"delegate\"}}\n",
                "{\"type\":\"message\",\"id\":\"assistant-1\",\"parentId\":\"user-1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"name\":\"worker_start\"}]}}\n"
            ),
        )?;
        assert_eq!(
            parent_before_worker_call(temp.path())?.as_deref(),
            Some("user-1")
        );
        Ok(())
    }

    #[test]
    fn worker_input_maps_only_interactive_requests() {
        let (input, kind) = worker_input(ExtensionUiRequest::Confirm {
            id: "confirm-1".into(),
            title: "Proceed?".into(),
            message: "This changes files".into(),
            timeout: None,
        })
        .expect("supported interaction")
        .expect("worker input");
        assert_eq!(input.id, "confirm-1");
        assert_eq!(input.options, ["Yes", "No"]);
        assert!(matches!(kind, InputKind::Confirm));
        assert!(
            worker_input(ExtensionUiRequest::Notify {
                id: "notice".into(),
                message: "done".into(),
                tone: crate::agents::extensions::NotifyTone::Info,
            })
            .expect("non-interactive request")
            .is_none()
        );
        assert!(
            worker_input(ExtensionUiRequest::Unknown {
                id: None,
                method: "future_prompt".into(),
            })
            .is_err()
        );
    }
}
