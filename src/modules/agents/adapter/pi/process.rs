//! One owned Pi RPC child process with strict framing and correlation.

use std::{
    collections::{HashMap, VecDeque},
    io::Write as _,
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
    wire::{PiResponse, PiWireMessage, parse_frame},
};
use crate::{
    access,
    agents::{
        FileAccessMode, NetworkAccessMode, PermissionLevel, PiEvent, PiRequest, encode_pi_request,
    },
    protocol::ExtensionUiResponse,
};

#[derive(Clone)]
pub(crate) struct PiProcessCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub permission_level: PermissionLevel,
    pub nono: access::NonoExecutable,
    pub grants: Option<access::GrantStore>,
    pub app_proxy: Option<String>,
}

impl Default for PiProcessCommand {
    fn default() -> Self {
        Self {
            program: pi_program(std::env::var_os("FARCASTER_PI_PATH")),
            prefix_args: Vec::new(),
            permission_level: PermissionLevel::default(),
            nono: access::configured_nono_program(std::env::var_os("FARCASTER_NONO_PATH")),
            grants: None,
            app_proxy: None,
        }
    }
}

impl PiProcessCommand {
    #[cfg(test)]
    pub(crate) fn test_script(script: &Path, mut arguments: Vec<String>) -> Self {
        let mut prefix_args = Vec::with_capacity(arguments.len() + 1);
        prefix_args.push(script.to_string_lossy().into_owned());
        prefix_args.append(&mut arguments);
        Self {
            program: PathBuf::from("sh"),
            prefix_args,
            permission_level: PermissionLevel::default(),
            nono: access::test_nono_bypass(),
            grants: None,
            app_proxy: None,
        }
    }

    pub(crate) fn command(&self, project: &Path) -> Result<access::PreparedCommand, String> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut environment = crate::shell_environment::project_shell_environment(project)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let mut environment: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>> = None;
        let environment_value = |name: &str| {
            environment
                .as_ref()
                .and_then(|values| values.iter().find(|(key, _)| key == name))
                .map(|(_, value)| value.clone())
                .or_else(|| std::env::var_os(name))
        };
        let environment_path = |name: &str| environment_value(name).map(PathBuf::from);
        let home = environment_path("HOME")
            .ok_or_else(|| "HOME is required to compile the Farcaster sandbox policy".to_owned())?;
        let agent_state =
            environment_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".pi/agent"));
        let temporary = environment_path("TMPDIR").unwrap_or_else(std::env::temp_dir);
        let access = sandbox_access(self.permission_level);
        if let Some(grants) = &self.grants {
            grants.set_access(access.filesystem, access.network);
        }
        let program =
            resolve_agent_program(&self.program, project, environment_value("PATH").as_deref())?;
        let network = access::network_configuration(
            environment.as_deref(),
            self.app_proxy.as_deref(),
            matches!(access.network, access::NetworkAccess::Sandboxed),
        )?;
        if let Some(environment) = environment.as_mut() {
            access::append_app_proxy_environment(environment, &network);
        }
        let mut prepared = access::prepare_command(
            &self.nono,
            &program,
            &self.prefix_args,
            access::PolicyPaths {
                project,
                home: &home,
                agent_state: &agent_state,
                temporary: &temporary,
            },
            access,
            self.grants.as_ref(),
            &network,
        )?;
        prepared.command.current_dir(project);
        if let Some(environment) = environment {
            prepared.command.env_clear().envs(environment);
        }
        Ok(prepared)
    }
}

fn sandbox_access(level: PermissionLevel) -> access::AccessPolicy {
    let filesystem = match level.files {
        FileAccessMode::ReadOnly => access::FilesystemAccess::ReadOnly,
        FileAccessMode::Sandboxed => access::FilesystemAccess::Sandboxed,
        FileAccessMode::Full => access::FilesystemAccess::Full,
    };
    let network = match level.network {
        NetworkAccessMode::Sandboxed => access::NetworkAccess::Sandboxed,
        NetworkAccessMode::Full => access::NetworkAccess::Full,
    };
    access::AccessPolicy {
        filesystem,
        network,
    }
}

fn pi_program(packaged_path: Option<std::ffi::OsString>) -> PathBuf {
    packaged_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("pi"))
}

fn resolve_agent_program(
    program: &Path,
    working_directory: &Path,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    let candidate = if program.is_absolute() {
        Some(program.to_owned())
    } else if program
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        Some(working_directory.join(program))
    } else {
        search_path.and_then(|search_path| {
            std::env::split_paths(search_path)
                .map(|directory| directory.join(program))
                .find(|candidate| is_executable_file(candidate))
        })
    };
    let candidate = candidate.ok_or_else(|| {
        format!(
            "agent executable was not found in the captured PATH: {}",
            program.display()
        )
    })?;
    if !is_executable_file(&candidate) {
        return Err(format!(
            "agent executable is not an executable file: {}",
            candidate.display()
        ));
    }
    candidate
        .canonicalize()
        .map_err(|error| format!("resolve agent executable {}: {error}", candidate.display()))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
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
    New,
    Resume(&'a Path),
    Fork(&'a Path),
}

fn rpc_command(
    command: &PiProcessCommand,
    project: &Path,
    launch: SessionLaunch<'_>,
    mcp_config: &Path,
) -> Result<access::PreparedCommand, String> {
    let mut prepared = command.command(project)?;
    prepared
        .command
        .args(["--mode", "rpc"])
        .arg("--mcp-config")
        .arg(mcp_config)
        // Farcaster owns the complete outer sandbox and access-request boundary.
        .env("PI_NONO_DISABLED", "1")
        .env("FARCASTER_NATIVE_NOTIFICATIONS", "1")
        // Compatibility with existing Pi notification extensions.
        .env("PI_GPUI_NATIVE_NOTIFICATIONS", "1");
    match launch {
        SessionLaunch::New => {}
        SessionLaunch::Resume(session) => {
            prepared.command.arg("--session").arg(session);
        }
        SessionLaunch::Fork(source) => {
            prepared.command.arg("--fork").arg(source);
        }
    }
    Ok(prepared)
}

pub(crate) struct PiRpcProcess {
    caller_identity: crate::workers::CallerIdentity,
    _mcp_config: TransientMcpConfig,
    _sandbox: access::PreparedCommand,
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    incoming: mpsc::Receiver<ReaderItem>,
    queued: VecDeque<PiEvent>,
    pending: HashMap<String, String>,
    next_id: u64,
    stderr: String,
}

impl PiRpcProcess {
    pub(crate) fn spawn(
        command: &PiProcessCommand,
        project: &Path,
        session: Option<&Path>,
    ) -> Result<Self, String> {
        let launch = session.map_or(SessionLaunch::New, SessionLaunch::Resume);
        Self::spawn_inner(command, project, launch, None)
    }

    pub(crate) fn spawn_with_waker(
        command: &PiProcessCommand,
        project: &Path,
        session: Option<&Path>,
        wake: thread::Thread,
    ) -> Result<Self, String> {
        let launch = session.map_or(SessionLaunch::New, SessionLaunch::Resume);
        Self::spawn_inner(command, project, launch, Some(wake))
    }

    pub(crate) fn spawn_fork(
        command: &PiProcessCommand,
        project: &Path,
        source: &Path,
    ) -> Result<Self, String> {
        Self::spawn_inner(command, project, SessionLaunch::Fork(source), None)
    }

    pub(crate) fn spawn_fork_with_waker(
        command: &PiProcessCommand,
        project: &Path,
        source: &Path,
        wake: thread::Thread,
    ) -> Result<Self, String> {
        Self::spawn_inner(command, project, SessionLaunch::Fork(source), Some(wake))
    }

    fn spawn_inner(
        command: &PiProcessCommand,
        project: &Path,
        launch: SessionLaunch<'_>,
        wake: Option<thread::Thread>,
    ) -> Result<Self, String> {
        let caller_identity = crate::workers::CallerRegistry::shared().issue();
        let mcp_config = TransientMcpConfig::create(caller_identity.token())?;
        let mut prepared = rpc_command(command, project, launch, mcp_config.path())?;
        let mut child = prepared
            .command
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
            _sandbox: prepared,
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            incoming,
            queued: VecDeque::new(),
            pending: HashMap::new(),
            next_id: 0,
            stderr: String::new(),
        };
        rpc.readiness_handshake(Duration::from_secs(15))?;
        Ok(rpc)
    }

    pub(crate) fn send_request(&mut self, request: PiRequest) -> Result<String, String> {
        self.send_command(encode_pi_request(request))
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

    pub(crate) fn request_and_wait(&mut self, request: PiRequest) -> Result<PiResponse, String> {
        let operation = request.operation();
        let id = self.send_request(request)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(50)) {
                Ok(ReaderItem::StderrEof) => {}
                Ok(item) => match self.route(item) {
                    PiEvent::Response(response) if response.id.as_deref() == Some(&id) => {
                        return if response.success {
                            Ok(response)
                        } else {
                            Err(response
                                .error
                                .unwrap_or_else(|| format!("Pi could not {operation}")))
                        };
                    }
                    PiEvent::Failure(error) => return Err(error),
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
        command: &PiProcessCommand,
        project: &Path,
        session: &Path,
        name: &str,
    ) -> Result<(), String> {
        let mut rpc = Self::spawn(command, project, Some(session))?;
        let result = (|| {
            let id = rpc.send_request(PiRequest::Rename {
                name: name.to_owned(),
            })?;
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                match rpc.try_next() {
                    Some(PiEvent::Response(response)) if response.id.as_deref() == Some(&id) => {
                        return if response.success {
                            Ok(())
                        } else {
                            Err(response
                                .error
                                .unwrap_or_else(|| "Pi rejected the session name".to_owned()))
                        };
                    }
                    Some(PiEvent::Failure(error)) => return Err(error),
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

    pub(crate) fn try_next(&mut self) -> Option<PiEvent> {
        if let Some(item) = self.queued.pop_front() {
            return Some(item);
        }
        match self.incoming.try_recv() {
            Ok(ReaderItem::StderrEof) => None,
            Ok(ReaderItem::Eof) => Some(self.finish_after_stdout_eof()),
            Ok(item) => Some(self.route(item)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(PiEvent::Failure("Pi reader threads stopped".into()))
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
        let id = self.send_request(PiRequest::LoadState)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(50)) {
                Ok(ReaderItem::StderrEof) => continue,
                Ok(item) => match self.route(item) {
                    PiEvent::Response(response) if response.id.as_deref() == Some(&id) => {
                        if response.command != "get_state" {
                            return Err(format!(
                                "readiness response command was {}",
                                response.command
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
                    PiEvent::Failure(error) => return Err(error),
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

    fn route(&mut self, item: ReaderItem) -> PiEvent {
        match item {
            ReaderItem::Wire(Ok(PiWireMessage::Response(response))) => {
                let Some(id) = response.id.as_deref() else {
                    return PiEvent::Failure(format!(
                        "uncorrelated response for {}",
                        response.command
                    ));
                };
                let Some(expected_command) = self.pending.remove(id) else {
                    return PiEvent::Failure(format!("response used unknown request id {id}"));
                };
                if response.command != expected_command {
                    return PiEvent::Failure(format!(
                        "response {id} was for {}, expected {expected_command}",
                        response.command
                    ));
                }
                if response.success
                    && response.command == "get_state"
                    && let Some(session) = response.data["sessionFile"].as_str()
                {
                    self.caller_identity.bind(session);
                }
                PiEvent::Response(response)
            }
            ReaderItem::Wire(Ok(PiWireMessage::ExtensionUi(request))) => {
                PiEvent::Interaction(request)
            }
            ReaderItem::Wire(Ok(PiWireMessage::Event(event))) => PiEvent::Activity(event),
            ReaderItem::Wire(Err(error)) => PiEvent::Failure(error),
            ReaderItem::Stderr(chunk) => {
                self.stderr.push_str(&chunk);
                PiEvent::Stderr(chunk)
            }
            ReaderItem::Eof => self.finish_after_stdout_eof(),
            ReaderItem::StderrEof => PiEvent::Stderr(String::new()),
        }
    }

    fn finish_after_stdout_eof(&mut self) -> PiEvent {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(20)) {
                Ok(ReaderItem::Stderr(chunk)) => self.stderr.push_str(&chunk),
                Ok(ReaderItem::StderrEof) => break,
                Ok(ReaderItem::Wire(wire)) => {
                    self.queued.push_back(match wire {
                        Ok(PiWireMessage::Response(response)) => PiEvent::Response(response),
                        Ok(PiWireMessage::ExtensionUi(request)) => PiEvent::Interaction(request),
                        Ok(PiWireMessage::Event(event)) => PiEvent::Activity(event),
                        Err(error) => PiEvent::Failure(error),
                    });
                }
                Ok(ReaderItem::Eof) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let exit = self.exit_description();
        if self.pending.is_empty() {
            PiEvent::Failure(format!(
                "Pi closed stdout ({exit}). Stderr: {}",
                self.stderr
            ))
        } else {
            PiEvent::Failure(format!(
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

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
