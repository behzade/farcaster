//! One owned Pi RPC child process with strict framing and correlation.

use std::{
    collections::{HashMap, VecDeque},
    fs::OpenOptions,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{
    framing::{JsonlFramer, encode_json_line},
    mcp_config::TransientMcpConfig,
    wire::{PiWireMessage, parse_frame},
};
use crate::modules::agents::adapter::farcaster_mcp::INSTRUCTIONS;
#[cfg(test)]
use crate::modules::agents::adapter::process_command::resolve_agent_program;
use crate::{
    agents::extensions::ExtensionUiResponse,
    agents::{
        AgentLaunchConfig, HarnessAccessMode, PeerMessage, SessionCommand, SessionEvent,
        SessionResponse, WorkerActivityState, WorkerSendMode,
    },
};

impl Default for crate::agents::AgentLaunchConfig {
    fn default() -> Self {
        Self {
            program: pi_program(std::env::var_os("FARCASTER_PI_PATH")),
            prefix_args: Vec::new(),
            access_mode: HarnessAccessMode::default(),
            app_proxy: None,
            session_locator_root: None,
        }
    }
}

fn pi_program(packaged_path: Option<std::ffi::OsString>) -> PathBuf {
    packaged_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("pi"))
}

#[derive(Clone)]
struct ReaderSender {
    sender: mpsc::Sender<ReaderItem>,
    wake: Option<thread::Thread>,
}

impl ReaderSender {
    fn send(&self, item: ReaderItem) -> Result<(), ()> {
        self.sender.send(item).map_err(|_| ())?;
        if let Some(wake) = &self.wake {
            wake.unpark();
        }
        Ok(())
    }
}

enum ReaderItem {
    Wire(Result<PiWireMessage, String>),
    Stderr(String),
    StderrEof,
    Eof,
}

enum SessionLaunch<'a> {
    Catalog,
    Ephemeral,
    New,
    Resume(&'a Path),
    Fork(&'a Path),
}

fn rpc_command(
    command: &AgentLaunchConfig,
    project: &Path,
    launch: SessionLaunch<'_>,
    mcp_config: &Path,
) -> Result<std::process::Command, String> {
    let mut prepared = command.command(project)?;
    prepared
        .args(["--mode", "rpc"])
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--append-system-prompt")
        .arg(INSTRUCTIONS)
        .env("FARCASTER_NATIVE_NOTIFICATIONS", "1")
        // Compatibility with existing Pi notification extensions.
        .env("PI_GPUI_NATIVE_NOTIFICATIONS", "1");
    if matches!(command.access_mode, HarnessAccessMode::Full) {
        prepared.env("PI_NONO_DISABLED", "1");
    }
    match launch {
        SessionLaunch::Catalog | SessionLaunch::Ephemeral => {
            prepared.arg("--no-session");
        }
        SessionLaunch::New => {}
        SessionLaunch::Resume(session) => {
            prepared.arg("--session").arg(session);
        }
        SessionLaunch::Fork(source) => {
            prepared.arg("--fork").arg(source);
        }
    }
    Ok(prepared)
}

pub(crate) struct PiRpcProcess {
    caller_identity: crate::modules::agents::core::CallerIdentity,
    _mcp_config: TransientMcpConfig,
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    incoming: mpsc::Receiver<ReaderItem>,
    queued: VecDeque<SessionEvent>,
    pending: HashMap<String, String>,
    peer_messages: VecDeque<PeerMessage>,
    next_id: u64,
    activity: WorkerActivityState,
    stderr: String,
    parent_session: Option<String>,
    pending_parent_stamp: Option<PathBuf>,
}

impl PiRpcProcess {
    pub(in crate::modules::agents::adapter) fn spawn_catalog(
        command: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<Self, String> {
        Self::spawn_inner(command, project, SessionLaunch::Catalog, None, None, None)
    }

    pub(crate) fn spawn(
        command: &AgentLaunchConfig,
        project: &Path,
        session: Option<&Path>,
    ) -> Result<Self, String> {
        let launch = session.map_or(SessionLaunch::New, SessionLaunch::Resume);
        Self::spawn_inner(command, project, launch, None, None, None)
    }

    pub(in crate::modules::agents::adapter) fn spawn_with_optional_waker(
        command: &AgentLaunchConfig,
        project: &Path,
        session: Option<&Path>,
        wake: Option<thread::Thread>,
    ) -> Result<Self, String> {
        let launch = session.map_or(SessionLaunch::New, SessionLaunch::Resume);
        Self::spawn_inner(command, project, launch, wake, None, None)
    }

    pub(crate) fn spawn_fork(
        command: &AgentLaunchConfig,
        project: &Path,
        source: &Path,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            command,
            project,
            SessionLaunch::Fork(source),
            None,
            None,
            None,
        )
    }

    pub(in crate::modules::agents::adapter) fn spawn_fork_with_optional_waker(
        command: &AgentLaunchConfig,
        project: &Path,
        source: &Path,
        wake: Option<thread::Thread>,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            command,
            project,
            SessionLaunch::Fork(source),
            wake,
            None,
            None,
        )
    }

    pub(crate) fn spawn_worker(
        command: &AgentLaunchConfig,
        project: &Path,
        worker_id: String,
        worker_name: String,
        parent: Option<(String, String)>,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            command,
            project,
            SessionLaunch::New,
            None,
            Some((worker_id, worker_name)),
            parent,
        )
    }

    pub(crate) fn spawn_ephemeral_worker(
        command: &AgentLaunchConfig,
        project: &Path,
        worker_id: String,
        worker_name: String,
    ) -> Result<Self, String> {
        Self::spawn_inner(
            command,
            project,
            SessionLaunch::Ephemeral,
            None,
            Some((worker_id, worker_name)),
            None,
        )
    }

    fn spawn_inner(
        command: &AgentLaunchConfig,
        project: &Path,
        launch: SessionLaunch<'_>,
        wake: Option<thread::Thread>,
        worker: Option<(String, String)>,
        parent: Option<(String, String)>,
    ) -> Result<Self, String> {
        let registry = crate::modules::agents::core::CallerRegistry::shared();
        let profile = crate::modules::agents::core::CallerProfile {
            backend: "pi".into(),
            provider: None,
            model: None,
            effort: None,
        };
        let parent_worker_id = parent.as_ref().map(|(id, _)| id.clone());
        let parent_session = parent.map(|(_, session)| session);
        let caller_identity = if let Some((worker_id, worker_name)) = worker {
            registry.issue_as(
                project,
                profile,
                wake.clone(),
                worker_id,
                worker_name,
                parent_worker_id,
            )?
        } else {
            registry.issue(project, profile, wake.clone())
        };
        let mcp_config = TransientMcpConfig::create(caller_identity.token())?;
        let mut prepared = rpc_command(command, project, launch, mcp_config.path())?;
        let mut child = prepared
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                format!(
                    "start {} for {}: {error}",
                    command.program.display(),
                    project.display()
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Pi stdin was not piped".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Pi stdout was not piped".to_owned())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Pi stderr was not piped".to_owned())?;
        let child = Arc::new(Mutex::new(child));
        let (sender, incoming) = mpsc::channel();
        let sender = ReaderSender { sender, wake };
        spawn_stdout_reader(stdout, sender.clone());
        spawn_stderr_reader(stderr, sender);
        let mut rpc = Self {
            caller_identity,
            _mcp_config: mcp_config,
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            incoming,
            queued: VecDeque::new(),
            pending: HashMap::new(),
            peer_messages: VecDeque::new(),
            next_id: 0,
            activity: WorkerActivityState::Idle,
            stderr: String::new(),
            parent_session,
            pending_parent_stamp: None,
        };
        rpc.readiness_handshake(Duration::from_secs(15))?;
        Ok(rpc)
    }

    fn set_activity(&mut self, activity: WorkerActivityState) {
        self.activity = activity;
        self.caller_identity.set_activity(activity);
    }

    pub(crate) fn send_request(&mut self, request: SessionCommand) -> Result<String, String> {
        let starts_run = matches!(
            &request,
            SessionCommand::Prompt {
                mode: crate::protocol::PromptMode::Normal,
                ..
            }
        );
        match &request {
            SessionCommand::SelectModel { provider, model_id } => {
                self.caller_identity.select_model(provider, model_id);
            }
            SessionCommand::SelectReasoning { level } => {
                self.caller_identity.select_effort(level);
            }
            _ => {}
        }
        let id = self.send_command(super::protocol::encode_request(request))?;
        if starts_run {
            self.set_activity(WorkerActivityState::Starting);
        }
        Ok(id)
    }

    fn send_command(&mut self, mut command: Value) -> Result<String, String> {
        let object = command
            .as_object_mut()
            .ok_or_else(|| "RPC command must be an object".to_owned())?;
        let command_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "RPC command requires a string type".to_owned())?
            .to_owned();
        self.next_id = self.next_id.saturating_add(1);
        let id = format!("gpui-{}", self.next_id);
        object.insert("id".into(), Value::String(id.clone()));
        let encoded = encode_json_line(&command)
            .map_err(|error| format!("encode {command_type}: {error}"))?;
        self.pending.insert(id.clone(), command_type);
        if let Err(error) = self.write(&encoded) {
            self.pending.remove(&id);
            return Err(error);
        }
        Ok(id)
    }

    pub(crate) fn request_and_wait(
        &mut self,
        request: SessionCommand,
    ) -> Result<SessionResponse, String> {
        let operation = request.operation();
        let id = self.send_request(request)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(50)) {
                Ok(ReaderItem::StderrEof) => {}
                Ok(item) => match self.route(item) {
                    SessionEvent::Response(response) if response.id.as_deref() == Some(&id) => {
                        return if response.success {
                            Ok(response)
                        } else {
                            Err(response
                                .error
                                .unwrap_or_else(|| format!("Pi could not {operation}")))
                        };
                    }
                    SessionEvent::Failure(error) => return Err(error),
                    other => self.queued.push_back(other),
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(format!(
                        "Pi readers stopped while attempting to {operation}"
                    ));
                }
            }
        }
        Err(format!("Pi did not {operation} within 15 seconds"))
    }

    pub(crate) fn rename_session(
        command: &AgentLaunchConfig,
        project: &Path,
        session: &Path,
        name: &str,
    ) -> Result<(), String> {
        let mut rpc = Self::spawn(command, project, Some(session))?;
        let result = (|| {
            let id = rpc.send_request(SessionCommand::Rename {
                name: name.to_owned(),
            })?;
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                match rpc.try_next() {
                    Some(SessionEvent::Response(response))
                        if response.id.as_deref() == Some(&id) =>
                    {
                        return if response.success {
                            Ok(())
                        } else {
                            Err(response
                                .error
                                .unwrap_or_else(|| "Pi rejected the session name".to_owned()))
                        };
                    }
                    Some(SessionEvent::Failure(error)) => return Err(error),
                    Some(_) | None => thread::sleep(Duration::from_millis(10)),
                }
            }
            Err("timed out while setting the session name".to_owned())
        })();
        let termination = rpc.terminate();
        result.and(termination)
    }

    pub(crate) fn send_extension_response(
        &mut self,
        response: ExtensionUiResponse,
    ) -> Result<(), String> {
        let value = serde_json::to_value(response)
            .map_err(|error| format!("encode extension UI response: {error}"))?;
        let encoded = encode_json_line(&value)
            .map_err(|error| format!("encode extension UI response: {error}"))?;
        self.write(&encoded)
    }

    pub(crate) fn try_next(&mut self) -> Option<SessionEvent> {
        if let Some(item) = self.queued.pop_front() {
            return Some(item);
        }
        if let Some(message) = self.caller_identity.try_recv() {
            self.peer_messages.push_back(message);
        }
        if let Some(mode) = WorkerSendMode::for_peer(self.activity)
            && let Some(message) = self.peer_messages.pop_front()
        {
            let mode = match mode {
                WorkerSendMode::Prompt => crate::protocol::PromptMode::Normal,
                WorkerSendMode::Steer => crate::protocol::PromptMode::Steer,
                WorkerSendMode::Queue => unreachable!(),
            };
            if let Err(error) = self.send_request(SessionCommand::Prompt {
                mode,
                message: message.prompt(),
                images: Vec::new(),
            }) {
                return Some(SessionEvent::Failure(error));
            }
        }
        match self.incoming.try_recv() {
            Ok(ReaderItem::StderrEof) => None,
            Ok(ReaderItem::Eof) => Some(self.finish_after_stdout_eof()),
            Ok(item) => Some(self.route(item)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(SessionEvent::Failure("Pi reader threads stopped".into()))
            }
        }
    }

    pub(crate) fn terminate(&mut self) -> Result<(), String> {
        let pid = {
            let mut child = self
                .child
                .lock()
                .map_err(|_| "Pi process lock was poisoned".to_owned())?;
            if child
                .try_wait()
                .map_err(|error| format!("check Pi before terminate: {error}"))?
                .is_some()
            {
                return Ok(());
            }
            child.id()
        };
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let exited = self
                .child
                .lock()
                .map_err(|_| "Pi process lock was poisoned".to_owned())?
                .try_wait()
                .map_err(|error| format!("wait for Pi: {error}"))?
                .is_some();
            if exited {
                return Ok(());
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.child
            .lock()
            .map_err(|_| "Pi process lock was poisoned".to_owned())?
            .kill()
            .map_err(|error| format!("kill Pi after timeout: {error}"))?;
        let reap_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let exited = self
                .child
                .lock()
                .map_err(|_| "Pi process lock was poisoned".to_owned())?
                .try_wait()
                .map_err(|error| format!("reap Pi after kill: {error}"))?
                .is_some();
            if exited {
                return Ok(());
            }
            if Instant::now() >= reap_deadline {
                return Err("Pi did not exit after forced termination".to_owned());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn readiness_handshake(&mut self, timeout: Duration) -> Result<(), String> {
        let id = self.send_request(SessionCommand::LoadState)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(50)) {
                Ok(ReaderItem::StderrEof) => continue,
                Ok(item) => match self.route(item) {
                    SessionEvent::Response(response) if response.id.as_deref() == Some(&id) => {
                        if response.operation != crate::agents::SessionOperation::LoadState {
                            return Err(format!(
                                "readiness response was for {:?}",
                                response.operation
                            ));
                        }
                        if !response.success {
                            return Err(format!(
                                "Pi readiness failed: {}",
                                response.error.unwrap_or_default()
                            ));
                        }
                        return Ok(());
                    }
                    SessionEvent::Failure(error) => return Err(error),
                    other => self.queued.push_back(other),
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Pi readers stopped during readiness".into());
                }
            }
        }
        Err(format!(
            "Pi did not answer get_state within {} seconds. Stderr: {}",
            timeout.as_secs(),
            self.stderr
        ))
    }

    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| "Pi stdin lock was poisoned".to_owned())?;
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("write Pi stdin: {error}"))
    }

    fn route(&mut self, item: ReaderItem) -> SessionEvent {
        self.retry_parent_stamp();
        match item {
            ReaderItem::Wire(Ok(PiWireMessage::Response { response, command })) => {
                let Some(id) = response.id.as_deref() else {
                    return SessionEvent::Failure(format!("uncorrelated response for {command}"));
                };
                let Some(expected_command) = self.pending.remove(id) else {
                    return SessionEvent::Failure(format!("response used unknown request id {id}"));
                };
                if command != expected_command {
                    return SessionEvent::Failure(format!(
                        "response {id} was for {command}, expected {expected_command}"
                    ));
                }
                if !response.success
                    && matches!(
                        response.operation,
                        crate::agents::SessionOperation::Prompt(
                            crate::protocol::PromptMode::Normal
                        )
                    )
                {
                    self.set_activity(WorkerActivityState::Idle);
                }
                if response.success
                    && response.operation == crate::agents::SessionOperation::LoadState
                    && let Some(session) = response.data["sessionFile"].as_str()
                {
                    self.caller_identity.bind(session);
                    if self.parent_session.is_some() {
                        self.pending_parent_stamp = Some(PathBuf::from(session));
                        self.retry_parent_stamp();
                    }
                }
                SessionEvent::Response(response)
            }
            ReaderItem::Wire(Ok(PiWireMessage::ExtensionUi(request))) => {
                SessionEvent::Interaction(request)
            }
            ReaderItem::Wire(Ok(PiWireMessage::Event(event))) => {
                match event.get("type").and_then(Value::as_str) {
                    Some("agent_start") => self.set_activity(WorkerActivityState::Working),
                    Some("agent_settled") => self.set_activity(WorkerActivityState::Idle),
                    _ => {}
                }
                SessionEvent::Activity(event.into())
            }
            ReaderItem::Wire(Err(error)) => SessionEvent::Failure(error),
            ReaderItem::Stderr(chunk) => {
                self.stderr.push_str(&chunk);
                SessionEvent::Stderr(chunk)
            }
            ReaderItem::Eof => self.finish_after_stdout_eof(),
            ReaderItem::StderrEof => SessionEvent::Stderr(String::new()),
        }
    }

    fn retry_parent_stamp(&mut self) {
        let (Some(path), Some(parent)) = (
            self.pending_parent_stamp.as_deref(),
            self.parent_session.as_deref(),
        ) else {
            return;
        };
        if stamp_parent_session(path, parent).is_ok() {
            self.pending_parent_stamp = None;
            self.parent_session = None;
        }
    }

    fn finish_after_stdout_eof(&mut self) -> SessionEvent {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(20)) {
                Ok(ReaderItem::Stderr(chunk)) => self.stderr.push_str(&chunk),
                Ok(ReaderItem::StderrEof) => break,
                Ok(ReaderItem::Wire(wire)) => {
                    self.queued.push_back(match wire {
                        Ok(PiWireMessage::Response { response, .. }) => {
                            SessionEvent::Response(response)
                        }
                        Ok(PiWireMessage::ExtensionUi(request)) => {
                            SessionEvent::Interaction(request)
                        }
                        Ok(PiWireMessage::Event(event)) => SessionEvent::Activity(event.into()),
                        Err(error) => SessionEvent::Failure(error),
                    });
                }
                Ok(ReaderItem::Eof) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let exit = self.exit_description();
        if self.pending.is_empty() {
            SessionEvent::Failure(format!(
                "Pi closed stdout ({exit}). Stderr: {}",
                self.stderr
            ))
        } else {
            SessionEvent::Failure(format!(
                "Pi closed stdout with {} pending request(s), {exit}. Stderr: {}",
                self.pending.len(),
                self.stderr
            ))
        }
    }

    fn exit_description(&self) -> String {
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if let Ok(mut child) = self.child.lock()
                && let Ok(Some(status)) = child.try_wait()
            {
                return status.code().map_or_else(
                    || format!("process terminated by signal ({status})"),
                    |code| format!("exit code {code}"),
                );
            }
            if Instant::now() >= deadline {
                return "exit status not available".to_owned();
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for PiRpcProcess {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn spawn_stdout_reader(mut stdout: impl std::io::Read + Send + 'static, sender: ReaderSender) {
    thread::Builder::new()
        .name("farcaster-stdout".into())
        .spawn(move || {
            let mut framer = JsonlFramer::default();
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        for frame in framer.push(&buffer[..count]) {
                            if sender.send(ReaderItem::Wire(parse_frame(&frame))).is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let _ =
                            sender.send(ReaderItem::Wire(Err(format!("read Pi stdout: {error}"))));
                        return;
                    }
                }
            }
            if let Some(frame) = framer.finish() {
                let _ = sender.send(ReaderItem::Wire(parse_frame(&frame)));
            }
            let _ = sender.send(ReaderItem::Eof);
        })
        .ok();
}

fn spawn_stderr_reader(mut stderr: impl std::io::Read + Send + 'static, sender: ReaderSender) {
    thread::Builder::new()
        .name("farcaster-stderr".into())
        .spawn(move || {
            let mut buffer = [0_u8; 2 * 1024];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let chunk = String::from_utf8_lossy(&buffer[..count]).into_owned();
                        if sender.send(ReaderItem::Stderr(chunk)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ =
                            sender.send(ReaderItem::Stderr(format!("stderr read failed: {error}")));
                        break;
                    }
                }
            }
            let _ = sender.send(ReaderItem::StderrEof);
        })
        .ok();
}

fn stamp_parent_session(path: &Path, parent: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("open Pi child session {}: {error}", path.display()))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|error| format!("read Pi child session {}: {error}", path.display()))?;
    let (header_line, rest) = contents.split_once('\n').unwrap_or((contents.as_str(), ""));
    let mut header: Value = serde_json::from_str(header_line)
        .map_err(|error| format!("decode Pi child session header: {error}"))?;
    let object = header
        .as_object_mut()
        .filter(|header| header.get("type").and_then(Value::as_str) == Some("session"))
        .ok_or_else(|| format!("invalid Pi child session header: {}", path.display()))?;
    if object.get("parentSession").and_then(Value::as_str) == Some(parent) {
        return Ok(());
    }
    object.insert("parentSession".into(), Value::String(parent.to_owned()));
    let mut encoded = serde_json::to_string(&header)
        .map_err(|error| format!("encode Pi child session header: {error}"))?;
    encoded.push('\n');
    encoded.push_str(rest);
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek Pi child session: {error}"))?;
    file.write_all(encoded.as_bytes())
        .map_err(|error| format!("write Pi child session: {error}"))?;
    file.set_len(encoded.len() as u64)
        .map_err(|error| format!("truncate Pi child session: {error}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
