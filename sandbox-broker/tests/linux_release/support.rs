use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use pi_sandbox_broker::framing::{read_frame, write_frame};
use pi_sandbox_broker::protocol::{
    Access, ClientRequest, CommandSpec, DeniedAccess, DenyScope, ExecRequest, FilesystemDeny,
    FilesystemRight, MissingPathBehavior, NetworkPolicy, PathScope, SandboxPolicy, ServerEvent,
};

pub const RELEASE_TEST_TIMEOUT: Duration = Duration::from_secs(10);
const STARTED_MARKER: &[u8] = b"PI_RELEASE_READY\n";

pub struct TempRoot(pub PathBuf);

impl TempRoot {
    pub fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pi-linux-release-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create release test root");
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct CommandResult {
    pub output: Vec<u8>,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
enum StartAction {
    None,
    Cancel,
    Shutdown,
}

pub struct Broker {
    child: Child,
    input: BufWriter<ChildStdin>,
    events: Receiver<Result<ServerEvent, String>>,
}

impl Broker {
    pub fn start() -> Self {
        let mut child = broker_command()
            .env("PI_RELEASE_HOST_SENTINEL", "must-not-leak")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start packaged broker test binary");
        let input = BufWriter::new(child.stdin.take().expect("broker stdin"));
        let stdout = child.stdout.take().expect("broker stdout");
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            let mut output = BufReader::new(stdout);
            loop {
                match read_frame::<ServerEvent>(&mut output) {
                    Ok(Some(event)) => {
                        if sender.send(Ok(event)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {
                        let _ = sender.send(Err("broker closed its event pipe".to_owned()));
                        return;
                    }
                    Err(error) => {
                        let _ = sender.send(Err(format!("cannot decode broker event: {error}")));
                        return;
                    }
                }
            }
        });
        let broker = Self {
            child,
            input,
            events,
        };
        let ready = broker.next_event();
        assert!(
            matches!(
                ready,
                ServerEvent::Ready {
                    version: 4,
                    ref platform,
                    ref backend,
                    can_exec: true,
                    max_frame_bytes: 1_048_576,
                } if platform == "linux" && backend == "bubblewrap"
            ),
            "broker is not ready for the unsandboxed Linux release gate: {ready:?}"
        );
        broker
    }

    pub fn send(&mut self, request: &ClientRequest) {
        write_frame(&mut self.input, request).expect("write broker request");
    }

    pub fn exec(&mut self, request: ExecRequest) -> CommandResult {
        self.exec_inner(request, StartAction::None)
    }

    pub fn exec_and_cancel(&mut self, request: ExecRequest) -> CommandResult {
        self.exec_inner(request, StartAction::Cancel)
    }

    pub fn exec_and_shutdown(&mut self, request: ExecRequest) -> CommandResult {
        self.exec_inner(request, StartAction::Shutdown)
    }

    fn exec_inner(&mut self, request: ExecRequest, start_action: StartAction) -> CommandResult {
        let id = request.id.clone();
        self.send(&ClientRequest::Exec(request));
        let mut started = false;
        let mut action_sent = matches!(start_action, StartAction::None);
        let mut output = Vec::new();
        loop {
            match self.next_event() {
                ServerEvent::Started { id: event_id, .. } if event_id == id => started = true,
                ServerEvent::Stdout {
                    id: event_id,
                    data_base64,
                    ..
                }
                | ServerEvent::Stderr {
                    id: event_id,
                    data_base64,
                    ..
                } if event_id == id => {
                    output.extend(BASE64.decode(data_base64).expect("base64 child output"));
                    if !action_sent
                        && output
                            .windows(STARTED_MARKER.len())
                            .any(|window| window == STARTED_MARKER)
                    {
                        match start_action {
                            StartAction::None => unreachable!("no action was already marked sent"),
                            StartAction::Cancel => {
                                self.send(&ClientRequest::Cancel { id: id.clone() });
                            }
                            StartAction::Shutdown => self.send(&ClientRequest::Shutdown),
                        }
                        action_sent = true;
                    }
                }
                ServerEvent::Exit {
                    id: event_id,
                    code,
                    timed_out,
                    cancelled,
                    output_truncated,
                    ..
                } if event_id == id => {
                    assert!(started, "exit arrived before started");
                    assert!(action_sent, "command exited before its user-code marker");
                    return CommandResult {
                        output,
                        code,
                        timed_out,
                        cancelled,
                        truncated: output_truncated,
                    };
                }
                ServerEvent::Denials { id: event_id, .. } if event_id == id => {
                    panic!("Linux broker unexpectedly emitted denial hints")
                }
                ServerEvent::Error {
                    id: Some(event_id),
                    code,
                    message,
                } if event_id == id => panic!("broker rejected release test: {code:?}: {message}"),
                other => panic!("unexpected broker event: {other:?}"),
            }
        }
    }

    pub fn wait_for_exit(&mut self) {
        let deadline = Instant::now() + RELEASE_TEST_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("wait for broker") {
                assert!(status.success(), "broker shutdown status: {status}");
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("broker did not exit after shutdown");
    }

    fn next_event(&self) -> ServerEvent {
        self.events
            .recv_timeout(RELEASE_TEST_TIMEOUT)
            .expect("broker event timed out")
            .unwrap_or_else(|message| panic!("broker event stream failed: {message}"))
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = write_frame(&mut self.input, &ClientRequest::Shutdown);
        let deadline = Instant::now() + RELEASE_TEST_TIMEOUT;
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn broker_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pi-sandbox-broker"))
}

pub fn request(
    id: &str,
    workspace: &Path,
    script: String,
    grants: Vec<FilesystemRight>,
    denies: Vec<FilesystemDeny>,
    timeout_ms: Option<u64>,
    output_limit_bytes: u64,
) -> ExecRequest {
    ExecRequest {
        id: id.to_owned(),
        command: CommandSpec {
            program: shell_program(),
            args: vec!["-c".to_owned(), script],
        },
        cwd: workspace.to_string_lossy().into_owned(),
        env: BTreeMap::from([
            ("HOME".to_owned(), workspace.to_string_lossy().into_owned()),
            ("ONLY".to_owned(), "yes".to_owned()),
            (
                "PATH".to_owned(),
                std::env::var("PATH").expect("release gate PATH"),
            ),
        ]),
        timeout_ms,
        interactive: false,
        policy: SandboxPolicy {
            base_rights: vec![
                tree_right(Access::Read, Path::new("/")),
                tree_right(Access::Write, workspace),
            ],
            grants,
            denies,
            network: NetworkPolicy::Blocked,
            unix_socket_roots: vec![],
            output_limit_bytes,
        },
    }
}

pub fn tree_right(access: Access, path: &Path) -> FilesystemRight {
    FilesystemRight {
        access,
        path: path.to_string_lossy().into_owned(),
        scope: PathScope::Tree,
        missing_path: MissingPathBehavior::Reject,
    }
}

pub fn file_grant(path: &Path) -> FilesystemRight {
    FilesystemRight {
        access: Access::Write,
        path: path.to_string_lossy().into_owned(),
        scope: PathScope::File,
        missing_path: if path.exists() {
            MissingPathBehavior::Reject
        } else {
            MissingPathBehavior::CreateFile
        },
    }
}

pub fn tree_grant(path: &Path) -> FilesystemRight {
    FilesystemRight {
        access: Access::Write,
        path: path.to_string_lossy().into_owned(),
        scope: PathScope::Tree,
        missing_path: if path.exists() {
            MissingPathBehavior::Reject
        } else {
            MissingPathBehavior::CreateTree
        },
    }
}

pub fn file_deny(access: DeniedAccess, path: &Path) -> FilesystemDeny {
    FilesystemDeny {
        access,
        pattern: path.to_string_lossy().into_owned(),
        scope: DenyScope::File,
    }
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn find_program(name: &str) -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|root| root.join(name))
                .find(|candidate| candidate.is_file())
        })
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| panic!("release gate requires {name} on PATH"))
}

fn shell_program() -> String {
    find_program("bash").to_string_lossy().into_owned()
}

pub fn wait_for_path_absence(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !path.exists(),
        "temporary sandbox path was not removed: {}",
        path.display()
    );
}

pub fn assert_no_survivor(trigger: &Path, marker: &Path) {
    fs::write(trigger, "release").expect("release detached-process fixture");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        assert!(
            !marker.exists(),
            "detached sandbox process survived and created {}",
            marker.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

pub fn write_invalid_empty_frame(child: &mut Child) {
    let mut input = child.stdin.take().expect("broker stdin");
    input
        .write_all(&0_u32.to_be_bytes())
        .expect("write empty frame");
    input.flush().expect("flush empty frame");
}
