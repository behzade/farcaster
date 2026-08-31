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
    access::PreparedCommand,
    agents::AgentProcessCommand,
    modules::agents::adapter::farcaster_mcp,
    workers::{
        WorkerContext, WorkerEvent, WorkerInput, WorkerInputResponse, WorkerLaunch, WorkerSendMode,
        WorkerSession, WorkerSessionFactory,
    },
};

#[derive(Clone)]
pub(crate) struct CodexWorkerFactory {
    command: AgentProcessCommand,
}

impl CodexWorkerFactory {
    pub(crate) fn new(command: AgentProcessCommand) -> Self {
        Self { command }
    }
}

impl WorkerSessionFactory for CodexWorkerFactory {
    fn create(&self, launch: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
        if launch.provider.is_some() != launch.model.is_some() {
            return Err("Codex worker provider and model must be supplied together".into());
        }
        let mut sandbox = self.command.command(&launch.project)?;
        let caller_identity = crate::workers::CallerRegistry::shared().issue();
        configure_farcaster_mcp(&mut sandbox.command, caller_identity.token());
        let mut child = sandbox
            .command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("start Codex worker app-server: {error}"))?;
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
            _caller_identity: caller_identity,
            _sandbox: sandbox,
            child,
            writer,
            incoming,
            thread_id: thread_id.clone(),
            effort: launch.effort,
            next_id,
            current_turn: None,
            output: String::new(),
            pending: HashMap::new(),
            pending_inputs: HashMap::new(),
            events: VecDeque::from([WorkerEvent::SessionChanged { locator: thread_id }]),
        }))
    }
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

#[derive(Clone, Copy)]
enum PendingRequest {
    StartTurn,
    Ignore,
}

struct CodexWorkerSession {
    _caller_identity: crate::workers::CallerIdentity,
    _sandbox: PreparedCommand,
    child: Child,
    writer: ChildStdin,
    incoming: mpsc::Receiver<Result<CodexInbound, String>>,
    thread_id: String,
    effort: Option<String>,
    next_id: i64,
    current_turn: Option<String>,
    output: String,
    pending: HashMap<CodexRequestId, PendingRequest>,
    pending_inputs: HashMap<String, CodexRequestId>,
    events: VecDeque<WorkerEvent>,
}

impl WorkerSession for CodexWorkerSession {
    fn send(&mut self, message: String, mode: WorkerSendMode) -> Result<(), String> {
        let input = vec![CodexUserInput::text(message)];
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
        self.output.clear();
        let id = self.request(
            "turn/start",
            json!({
                "threadId": self.thread_id,
                "input": input,
                "effort": self.effort,
            }),
        )?;
        self.pending.insert(id, PendingRequest::StartTurn);
        Ok(())
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

    fn poll(&mut self) -> Option<WorkerEvent> {
        if let Some(event) = self.events.pop_front() {
            return Some(event);
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
                                self.current_turn = Some(turn_id.to_owned());
                            }
                        }
                        "item/agentMessage/delta" => {
                            if let Some(delta) = params["delta"].as_str() {
                                self.output.push_str(delta);
                            }
                        }
                        "turn/completed" => {
                            self.current_turn = None;
                            if params["turn"]["status"].as_str() == Some("failed") {
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

fn configure_farcaster_mcp(command: &mut std::process::Command, caller_token: &str) {
    let url = serde_json::to_string(farcaster_mcp::URL).expect("static MCP URL encodes");
    let header =
        serde_json::to_string(farcaster_mcp::CALLER_HEADER).expect("static MCP header encodes");
    let token = serde_json::to_string(caller_token).expect("caller token encodes");
    command
        .args(["app-server", "--stdio"])
        .arg("-c")
        .arg(format!("mcp_servers.farcaster.url={url}"))
        .arg("-c")
        .arg(format!(
            "mcp_servers.farcaster.http_headers={{{header}={token}}}"
        ))
        .arg("-c")
        .arg("mcp_servers.farcaster.required=true");
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
    fn native_startup_configures_required_farcaster_mcp() {
        let mut command = std::process::Command::new("codex");
        configure_farcaster_mcp(&mut command, "caller-1");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(&arguments[..2], ["app-server", "--stdio"]);
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
