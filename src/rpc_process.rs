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

use crate::{
    backend::{BackendEvent, BackendRequest, encode_pi_request},
    framing::{JsonlFramer, encode_json_line},
    protocol::{ExtensionUiResponse, WireMessage, parse_frame},
    runtime::PermissionLevel,
};

#[derive(Clone)]
pub(crate) struct ProcessCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub permission_level: PermissionLevel,
    pub nono: crate::sandbox::NonoExecutable,
    pub grants: Option<crate::sandbox::GrantStore>,
    pub app_proxy: Option<String>,
}

impl Default for ProcessCommand {
    fn default() -> Self {
        Self {
            program: pi_program(std::env::var_os("FARCASTER_PI_PATH")),
            prefix_args: Vec::new(),
            permission_level: PermissionLevel::default(),
            nono: crate::sandbox::configured_nono_program(std::env::var_os("FARCASTER_NONO_PATH")),
            grants: None,
            app_proxy: None,
        }
    }
}

impl ProcessCommand {
    #[cfg(test)]
    pub(crate) fn test_script(script: &Path, mut arguments: Vec<String>) -> Self {
        let mut prefix_args = Vec::with_capacity(arguments.len() + 1);
        prefix_args.push(script.to_string_lossy().into_owned());
        prefix_args.append(&mut arguments);
        Self {
            program: PathBuf::from("sh"),
            prefix_args,
            permission_level: PermissionLevel::default(),
            nono: crate::sandbox::test_nono_bypass(),
            grants: None,
            app_proxy: None,
        }
    }

    pub(crate) fn command(
        &self,
        project: &Path,
    ) -> Result<crate::sandbox::PreparedCommand, String> {
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
        let network = crate::network::configuration(
            environment.as_deref(),
            self.app_proxy.as_deref(),
            matches!(access.network, crate::sandbox::NetworkAccess::Sandboxed),
        )?;
        if let Some(environment) = environment.as_mut() {
            crate::network::append_app_proxy_environment(environment, &network);
        }
        let mut prepared = crate::sandbox::prepare_command(
            &self.nono,
            &program,
            &self.prefix_args,
            crate::sandbox::PolicyPaths {
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

fn sandbox_access(level: PermissionLevel) -> crate::sandbox::AccessPolicy {
    let filesystem = match level.files {
        crate::runtime::FileAccessMode::ReadOnly => crate::sandbox::FilesystemAccess::ReadOnly,
        crate::runtime::FileAccessMode::Sandboxed => crate::sandbox::FilesystemAccess::Sandboxed,
        crate::runtime::FileAccessMode::Full => crate::sandbox::FilesystemAccess::Full,
    };
    let network = match level.network {
        crate::runtime::NetworkAccessMode::Sandboxed => crate::sandbox::NetworkAccess::Sandboxed,
        crate::runtime::NetworkAccessMode::Full => crate::sandbox::NetworkAccess::Full,
    };
    crate::sandbox::AccessPolicy {
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
    Wire(Result<WireMessage, String>),
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
    command: &ProcessCommand,
    project: &Path,
    launch: SessionLaunch<'_>,
    mcp_config: &Path,
) -> Result<crate::sandbox::PreparedCommand, String> {
    let mut prepared = command.command(project)?;
    prepared
        .command
        .args(["--mode", "rpc"])
        // Pi and pi-nono run unrestricted inside Farcaster's outer sandbox.
        .args(["--sandbox-files", "full", "--sandbox-network", "full"])
        .arg("--mcp-config")
        .arg(mcp_config)
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

pub(crate) struct RpcProcess {
    _mcp_config: crate::mcp_client_config::TransientMcpConfig,
    _sandbox: crate::sandbox::PreparedCommand,
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    incoming: mpsc::Receiver<ReaderItem>,
    queued: VecDeque<BackendEvent>,
    pending: HashMap<String, String>,
    next_id: u64,
    stderr: String,
}

impl RpcProcess {
    pub(crate) fn spawn(
        command: &ProcessCommand,
        project: &Path,
        session: Option<&Path>,
    ) -> Result<Self, String> {
        let launch = session.map_or(SessionLaunch::New, SessionLaunch::Resume);
        Self::spawn_inner(command, project, launch, None)
    }

    pub(crate) fn spawn_with_waker(
        command: &ProcessCommand,
        project: &Path,
        session: Option<&Path>,
        wake: thread::Thread,
    ) -> Result<Self, String> {
        let launch = session.map_or(SessionLaunch::New, SessionLaunch::Resume);
        Self::spawn_inner(command, project, launch, Some(wake))
    }

    pub(crate) fn spawn_fork_with_waker(
        command: &ProcessCommand,
        project: &Path,
        source: &Path,
        wake: thread::Thread,
    ) -> Result<Self, String> {
        Self::spawn_inner(command, project, SessionLaunch::Fork(source), Some(wake))
    }

    fn spawn_inner(
        command: &ProcessCommand,
        project: &Path,
        launch: SessionLaunch<'_>,
        wake: Option<thread::Thread>,
    ) -> Result<Self, String> {
        let mcp_config = crate::mcp_client_config::TransientMcpConfig::create()?;
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

    pub(crate) fn send_request(&mut self, request: BackendRequest) -> Result<String, String> {
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

    pub(crate) fn rename_session(
        command: &ProcessCommand,
        project: &Path,
        session: &Path,
        name: &str,
    ) -> Result<(), String> {
        let mut rpc = Self::spawn(command, project, Some(session))?;
        let result = (|| {
            let id = rpc.send_request(BackendRequest::Rename {
                name: name.to_owned(),
            })?;
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                match rpc.try_next() {
                    Some(BackendEvent::Response(response))
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
                    Some(BackendEvent::Failure(error)) => return Err(error),
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

    pub(crate) fn try_next(&mut self) -> Option<BackendEvent> {
        if let Some(item) = self.queued.pop_front() {
            return Some(item);
        }
        match self.incoming.try_recv() {
            Ok(ReaderItem::StderrEof) => None,
            Ok(ReaderItem::Eof) => Some(self.finish_after_stdout_eof()),
            Ok(item) => Some(self.route(item)),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(BackendEvent::Failure("Pi reader threads stopped".into()))
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
        let id = self.send_request(BackendRequest::LoadState)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(50)) {
                Ok(ReaderItem::StderrEof) => continue,
                Ok(item) => match self.route(item) {
                    BackendEvent::Response(response) if response.id.as_deref() == Some(&id) => {
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
                    BackendEvent::Failure(error) => return Err(error),
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

    fn route(&mut self, item: ReaderItem) -> BackendEvent {
        match item {
            ReaderItem::Wire(Ok(WireMessage::Response(response))) => {
                let Some(id) = response.id.as_deref() else {
                    return BackendEvent::Failure(format!(
                        "uncorrelated response for {}",
                        response.command
                    ));
                };
                let Some(expected_command) = self.pending.remove(id) else {
                    return BackendEvent::Failure(format!("response used unknown request id {id}"));
                };
                if response.command != expected_command {
                    return BackendEvent::Failure(format!(
                        "response {id} was for {}, expected {expected_command}",
                        response.command
                    ));
                }
                BackendEvent::Response(response)
            }
            ReaderItem::Wire(Ok(WireMessage::ExtensionUi(request))) => {
                BackendEvent::Interaction(request)
            }
            ReaderItem::Wire(Ok(WireMessage::Event(event))) => BackendEvent::Activity(event),
            ReaderItem::Wire(Err(error)) => BackendEvent::Failure(error),
            ReaderItem::Stderr(chunk) => {
                self.stderr.push_str(&chunk);
                BackendEvent::Stderr(chunk)
            }
            ReaderItem::Eof => self.finish_after_stdout_eof(),
            ReaderItem::StderrEof => BackendEvent::Stderr(String::new()),
        }
    }

    fn finish_after_stdout_eof(&mut self) -> BackendEvent {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            match self.incoming.recv_timeout(Duration::from_millis(20)) {
                Ok(ReaderItem::Stderr(chunk)) => self.stderr.push_str(&chunk),
                Ok(ReaderItem::StderrEof) => break,
                Ok(ReaderItem::Wire(wire)) => {
                    self.queued.push_back(match wire {
                        Ok(WireMessage::Response(response)) => BackendEvent::Response(response),
                        Ok(WireMessage::ExtensionUi(request)) => BackendEvent::Interaction(request),
                        Ok(WireMessage::Event(event)) => BackendEvent::Activity(event),
                        Err(error) => BackendEvent::Failure(error),
                    });
                }
                Ok(ReaderItem::Eof) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let exit = self.exit_description();
        if self.pending.is_empty() {
            BackendEvent::Failure(format!(
                "Pi closed stdout ({exit}). Stderr: {}",
                self.stderr
            ))
        } else {
            BackendEvent::Failure(format!(
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

impl Drop for RpcProcess {
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
mod tests {
    use super::*;
    use crate::runtime::{FileAccessMode, NetworkAccessMode};
    use std::{error::Error, fs};
    use tempfile::tempdir;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn fake(case: &str) -> TestResult<(tempfile::TempDir, ProcessCommand)> {
        let temp = tempdir()?;
        let script = temp.path().join("fake.sh");
        fs::write(&script, include_str!("../tests/fixtures/fake-pi.sh"))?;
        let command = ProcessCommand::test_script(&script, vec![case.into()]);
        Ok((temp, command))
    }

    #[test]
    fn process_starts_directly_in_the_project_directory() -> TestResult {
        let (temp, command) = fake("project-directory")?;
        let mut rpc = RpcProcess::spawn(&command, temp.path(), None)?;
        let process_project = fs::read_to_string(temp.path().join("process-project"))?;
        assert_eq!(
            fs::canonicalize(process_project)?,
            fs::canonicalize(temp.path())?,
        );
        let mcp_config = serde_json::from_slice::<serde_json::Value>(&fs::read(
            temp.path().join("process-mcp-config"),
        )?)?;
        assert_eq!(
            mcp_config["mcpServers"]["farcaster"]["url"],
            "http://127.0.0.1:8765/mcp"
        );
        assert!(!temp.path().join(".mcp.json").exists());
        rpc.terminate()?;
        Ok(())
    }

    #[test]
    fn fork_process_passes_the_source_session_to_pi() -> TestResult {
        let project = tempdir()?;
        let source = Path::new("/sessions/source session.jsonl");
        let process = rpc_command(
            &ProcessCommand {
                nono: crate::sandbox::test_nono_bypass(),
                ..ProcessCommand::default()
            },
            project.path(),
            SessionLaunch::Fork(source),
            Path::new("/dev/fd/9"),
        )?;
        let arguments = process.command.get_args().collect::<Vec<_>>();
        assert!(arguments.windows(2).any(|pair| pair == ["--mode", "rpc"]));
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--mcp-config", "/dev/fd/9"])
        );
        assert_eq!(
            arguments.get(arguments.len().saturating_sub(2)..),
            Some([std::ffi::OsStr::new("--fork"), source.as_os_str()].as_slice())
        );
        Ok(())
    }

    #[test]
    fn packaged_pi_path_wins_over_the_project_environment() {
        assert_eq!(
            pi_program(Some("/nix/store/pi/bin/pi".into())),
            PathBuf::from("/nix/store/pi/bin/pi")
        );
        assert_eq!(pi_program(None), PathBuf::from("pi"));
    }

    #[cfg(unix)]
    #[test]
    fn resolves_agent_symlink_to_a_fixed_executable() -> TestResult {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = tempdir()?;
        let executable = root.path().join("agent");
        let bin = root.path().join("bin");
        fs::create_dir(&bin)?;
        fs::write(&executable, b"#!/usr/bin/env node\n")?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
        symlink(&executable, bin.join("agent"))?;

        let search_path = std::env::join_paths([&bin])?;
        let resolved = resolve_agent_program(Path::new("agent"), root.path(), Some(&search_path))?;
        assert_eq!(resolved, executable.canonicalize()?);
        Ok(())
    }

    #[test]
    fn pi_runs_full_inside_the_outer_sandbox() -> TestResult {
        let project = tempdir()?;
        let nono = project.path().join("nono");
        fs::write(&nono, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&nono, fs::Permissions::from_mode(0o700))?;
        }
        let pi = project.path().join("pi");
        fs::write(&pi, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&pi, fs::Permissions::from_mode(0o700))?;
        }
        let command = ProcessCommand {
            program: pi,
            permission_level: PermissionLevel {
                files: FileAccessMode::ReadOnly,
                network: NetworkAccessMode::Sandboxed,
            },
            nono: crate::sandbox::NonoExecutable::Fixed(nono.clone()),
            ..ProcessCommand::default()
        };
        let prepared = rpc_command(
            &command,
            project.path(),
            SessionLaunch::New,
            Path::new("/dev/fd/9"),
        )?;
        assert_eq!(prepared.command.get_program(), nono.as_os_str());
        let arguments = prepared.command.get_args().collect::<Vec<_>>();
        assert!(arguments.windows(4).any(|arguments| {
            arguments == ["--sandbox-files", "full", "--sandbox-network", "full"]
        }));
        Ok(())
    }

    #[test]
    fn handshake_routes_async_event_and_correlates_unique_ids() -> TestResult {
        let (temp, command) = fake("normal")?;
        let mut rpc = RpcProcess::spawn(&command, temp.path(), None)?;
        assert!(
            matches!(rpc.try_next(), Some(BackendEvent::Activity(value)) if value["type"] == "agent_start")
        );
        let first = rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
        let second = rpc.send_command(serde_json::json!({"type":"get_state"}))?;
        let stats = rpc.send_command(serde_json::json!({"type":"get_session_stats"}))?;
        assert_ne!(first, second);
        assert_ne!(second, stats);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut responses = 0;
        let mut context_shape = false;
        while Instant::now() < deadline && responses < 3 {
            if let Some(BackendEvent::Response(response)) = rpc.try_next() {
                responses += 1;
                context_shape |= response.command == "get_session_stats"
                    && response
                        .data
                        .pointer("/contextUsage/tokens")
                        .and_then(serde_json::Value::as_u64)
                        == Some(4096)
                    && response
                        .data
                        .pointer("/contextUsage/contextWindow")
                        .and_then(serde_json::Value::as_u64)
                        == Some(8192)
                    && response
                        .data
                        .pointer("/contextUsage/percent")
                        .and_then(serde_json::Value::as_u64)
                        == Some(50);
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(responses, 3);
        assert!(context_shape);
        Ok(())
    }

    #[test]
    fn eof_with_pending_request_is_failure_and_stderr_is_visible() -> TestResult {
        let (temp, command) = fake("eof")?;
        let mut rpc = RpcProcess::spawn(&command, temp.path(), None)?;
        rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut failure = String::new();
        while Instant::now() < deadline && failure.is_empty() {
            if let Some(BackendEvent::Failure(error)) = rpc.try_next() {
                failure = error;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(failure.contains("pending request"));
        assert!(failure.contains("exit code 7"));
        assert!(failure.contains("fake stderr before exit"));
        Ok(())
    }

    #[test]
    fn failed_readiness_is_reported() -> TestResult {
        let (temp, command) = fake("bad-handshake")?;
        let error = RpcProcess::spawn(&command, temp.path(), None)
            .err()
            .unwrap_or_default();
        assert!(error.contains("readiness"), "{error}");
        Ok(())
    }

    #[test]
    fn stdout_eof_waits_for_delayed_final_stderr() -> TestResult {
        let (temp, command) = fake("delayed-stderr")?;
        let mut rpc = RpcProcess::spawn(&command, temp.path(), None)?;
        rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut failure = String::new();
        while Instant::now() < deadline && failure.is_empty() {
            if let Some(BackendEvent::Failure(error)) = rpc.try_next() {
                failure = error;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(failure.contains("delayed final stderr"), "{failure}");
        assert!(failure.contains("exit code 8"), "{failure}");
        Ok(())
    }

    #[test]
    fn readiness_rejects_a_command_mismatch_for_the_right_id() -> TestResult {
        let (temp, command) = fake("mismatch-handshake")?;
        let error = RpcProcess::spawn(&command, temp.path(), None)
            .err()
            .unwrap_or_default();
        assert!(error.contains("expected get_state"));
        Ok(())
    }

    #[test]
    fn ordinary_response_rejects_a_command_mismatch_for_the_right_id() -> TestResult {
        let (temp, command) = fake("mismatch-response")?;
        let mut rpc = RpcProcess::spawn(&command, temp.path(), None)?;
        rpc.send_command(serde_json::json!({"type":"get_messages"}))?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut failure = String::new();
        while Instant::now() < deadline && failure.is_empty() {
            if let Some(BackendEvent::Failure(error)) = rpc.try_next() {
                failure = error;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(failure.contains("expected get_messages"));
        Ok(())
    }

    #[test]
    fn terminate_reaps_graceful_and_term_ignoring_children() -> TestResult {
        for case_name in ["normal", "ignore-term"] {
            let (temp, command) = fake(case_name)?;
            let mut rpc = RpcProcess::spawn(&command, temp.path(), None)?;
            let started = Instant::now();
            rpc.terminate()?;
            assert!(started.elapsed() < Duration::from_secs(3));
            assert!(
                rpc.child
                    .lock()
                    .map_err(|_| "poisoned")?
                    .try_wait()?
                    .is_some()
            );
        }
        Ok(())
    }
}
