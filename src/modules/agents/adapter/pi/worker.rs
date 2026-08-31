use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::process::{PiProcessCommand, PiRpcProcess};
use crate::{
    agents::extensions::{ExtensionUiRequest, ExtensionUiResponse, PromptMode, SessionState},
    agents::{
        PiEvent, PiRequest, WorkerContext, WorkerEvent, WorkerInput, WorkerInputResponse,
        WorkerLaunch, WorkerSendMode, WorkerSession, WorkerSessionFactory,
    },
};

#[derive(Clone)]
pub(crate) struct PiWorkerFactory {
    command: PiProcessCommand,
}

impl PiWorkerFactory {
    pub(crate) fn new(command: PiProcessCommand) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for PiWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("Pi worker provider and model must be supplied together".into());
        }
        let parent = canonical_session(&launch.parent_session, "parent")?;
        let mut process = match &launch.context {
            WorkerContext::Fresh => {
                let child = create_blank_child_session(&parent, &launch.project)?;
                match PiRpcProcess::spawn(&self.command, &launch.project, Some(&child)) {
                    Ok(process) => process,
                    Err(error) => {
                        let _ = std::fs::remove_file(child);
                        return Err(error);
                    }
                }
            }
            WorkerContext::Session { session_locator } => {
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
                    process.request_and_wait(PiRequest::ForkAt { entry_id })?;
                    process
                } else {
                    PiRpcProcess::spawn_fork(&self.command, &launch.project, &source)?
                }
            }
        };
        process.request_and_wait(PiRequest::ConfigureSteering)?;
        if let (Some(provider), Some(model_id)) = (launch.provider, launch.model) {
            process.request_and_wait(PiRequest::SelectModel { provider, model_id })?;
        }
        if let Some(level) = launch.effort {
            process.request_and_wait(PiRequest::SelectReasoning { level })?;
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
        self.process.send_request(PiRequest::Prompt {
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
        self.process.send_request(PiRequest::Abort)?;
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        loop {
            match self.process.try_next()? {
                PiEvent::Activity(event) => match event["type"].as_str() {
                    Some("agent_start") => {
                        self.settled = false;
                        self.latest_output.clear();
                        if let Err(error) = self.request_session_state() {
                            return Some(WorkerEvent::Failed(error));
                        }
                        return Some(WorkerEvent::Started);
                    }
                    Some("message_end") => {
                        if let Some(output) = final_assistant_text(event.get("message")) {
                            self.latest_output = output;
                        }
                    }
                    Some("agent_settled") => {
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
                PiEvent::Interaction(request) => match worker_input(request) {
                    Ok(Some((input, kind))) => {
                        self.pending_inputs.insert(input.id.clone(), kind);
                        return Some(WorkerEvent::NeedsInput(input));
                    }
                    Ok(None) => {}
                    Err(error) => return Some(WorkerEvent::Failed(error)),
                },
                PiEvent::Response(response) if !response.success => {
                    return Some(WorkerEvent::Failed(
                        response
                            .error
                            .unwrap_or_else(|| format!("Pi rejected {}", response.command)),
                    ));
                }
                PiEvent::Response(response)
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
                PiEvent::Failure(error) => return Some(WorkerEvent::Failed(error)),
                PiEvent::Response(_) | PiEvent::Stderr(_) => {}
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
            self.state_request = Some(self.process.send_request(PiRequest::LoadState)?);
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

fn create_blank_child_session(parent: &Path, project: &Path) -> Result<PathBuf, String> {
    let directory = parent
        .parent()
        .ok_or_else(|| "Pi worker parent session has no directory".to_owned())?;
    let now = time::OffsetDateTime::now_utc();
    let id = format!("farcaster-worker-{}", now.unix_timestamp_nanos());
    let path = directory.join(format!("{id}.jsonl"));
    let timestamp = now
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| format!("format Pi worker timestamp: {error}"))?;
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("create blank Pi worker session: {error}"))?;
    let header = serde_json::json!({
        "type": "session",
        "version": 3,
        "id": id,
        "timestamp": timestamp,
        "cwd": project,
        "parentSession": parent,
    });
    serde_json::to_writer(&mut file, &header)
        .map_err(|error| format!("encode blank Pi worker session: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("write blank Pi worker session: {error}"))?;
    file.persist_noclobber(&path).map_err(|error| {
        format!(
            "persist blank Pi worker session {}: {error}",
            path.display()
        )
    })?;
    Ok(path)
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
    fn blank_worker_session_records_native_parent_lineage() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::tempdir()?;
        let parent = temp.path().join("parent.jsonl");
        std::fs::write(
            &parent,
            "{\"type\":\"session\",\"version\":3,\"id\":\"parent\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"cwd\":\"/project\"}\n",
        )?;
        let child = create_blank_child_session(&parent, Path::new("/project"))?;
        let header: Value = serde_json::from_str(&std::fs::read_to_string(child)?)?;
        assert_eq!(header["parentSession"], parent.to_string_lossy().as_ref());
        assert_eq!(header["cwd"], "/project");
        Ok(())
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
