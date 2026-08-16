#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use pi_sandbox_broker::framing::{read_frame, write_frame};
use pi_sandbox_broker::protocol::{
    Access, ClientRequest, CommandSpec, Denial, DeniedAccess, DenyScope, ExecRequest,
    FilesystemDeny, FilesystemRight, MissingPathBehavior, NetworkPolicy, PathScope, SandboxPolicy,
    ServerEvent,
};

const RELEASE_TEST_TIMEOUT: Duration = Duration::from_secs(5);
const STARTED_MARKER: &[u8] = b"PI_RELEASE_READY\n";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("pi-broker-release-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create release test root");
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Broker {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
}

struct CommandResult {
    output: Vec<u8>,
    code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    truncated: bool,
    denials: Vec<Denial>,
}

#[derive(Clone, Copy)]
enum StartAction {
    None,
    Cancel,
    Shutdown,
}

impl Broker {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pi-sandbox-broker"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start packaged broker test binary");
        let input = BufWriter::new(child.stdin.take().expect("broker stdin"));
        let mut output = BufReader::new(child.stdout.take().expect("broker stdout"));
        let ready = read_frame::<ServerEvent>(&mut output)
            .expect("read ready frame")
            .expect("ready frame");
        assert!(
            matches!(
                ready,
                ServerEvent::Ready {
                    version: 4,
                    ref platform,
                    ref backend,
                    can_exec: true,
                    max_frame_bytes: 1_048_576,
                } if platform == "macos" && backend == "seatbelt"
            ),
            "broker is not ready for the unsandboxed macOS release gate: {ready:?}"
        );
        Self {
            child,
            input,
            output,
        }
    }

    fn send(&mut self, request: &ClientRequest) {
        write_frame(&mut self.input, request).expect("write broker request");
    }

    fn exec(&mut self, request: ExecRequest) -> CommandResult {
        self.exec_inner(request, StartAction::None)
    }

    fn exec_and_cancel(&mut self, request: ExecRequest) -> CommandResult {
        self.exec_inner(request, StartAction::Cancel)
    }

    fn exec_and_shutdown(&mut self, request: ExecRequest) -> CommandResult {
        self.exec_inner(request, StartAction::Shutdown)
    }

    fn exec_inner(&mut self, request: ExecRequest, start_action: StartAction) -> CommandResult {
        let id = request.id.clone();
        self.send(&ClientRequest::Exec(request));
        let mut started = false;
        let mut action_sent = matches!(start_action, StartAction::None);
        let mut output = Vec::new();
        let mut denials = None;
        loop {
            let event = read_frame::<ServerEvent>(&mut self.output)
                .expect("read broker event")
                .expect("broker event before EOF");
            match event {
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
                            StartAction::None => {
                                unreachable!("no start action was already marked sent")
                            }
                            StartAction::Cancel => {
                                self.send(&ClientRequest::Cancel { id: id.clone() });
                            }
                            StartAction::Shutdown => self.send(&ClientRequest::Shutdown),
                        }
                        action_sent = true;
                    }
                }
                ServerEvent::Denials {
                    id: event_id,
                    items,
                    complete,
                } if event_id == id => {
                    assert!(started, "denials arrived before started");
                    assert!(!complete, "macOS denial hints must stay incomplete");
                    assert!(denials.replace(items).is_none(), "duplicate denials");
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
                    let denials =
                        denials.expect("started command must emit denial hints before exit");
                    return CommandResult {
                        output,
                        code,
                        timed_out,
                        cancelled,
                        truncated: output_truncated,
                        denials,
                    };
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

    fn wait_for_exit(&mut self) {
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
}

impl Drop for Broker {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        self.send(&ClientRequest::Shutdown);
        let deadline = Instant::now() + RELEASE_TEST_TIMEOUT;
        while Instant::now() < deadline {
            if self.child.try_wait().expect("wait for broker").is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tree_right(access: Access, path: &Path) -> FilesystemRight {
    FilesystemRight {
        access,
        path: path.to_string_lossy().into_owned(),
        scope: PathScope::Tree,
        missing_path: MissingPathBehavior::Reject,
    }
}

fn file_grant(path: &Path) -> FilesystemRight {
    FilesystemRight {
        access: Access::Write,
        path: path.to_string_lossy().into_owned(),
        scope: PathScope::File,
        missing_path: MissingPathBehavior::CreateFile,
    }
}

fn tree_grant(path: &Path) -> FilesystemRight {
    FilesystemRight {
        access: Access::Write,
        path: path.to_string_lossy().into_owned(),
        scope: PathScope::Tree,
        missing_path: MissingPathBehavior::Reject,
    }
}

fn request(
    id: &str,
    workspace: &Path,
    script: String,
    grants: Vec<FilesystemRight>,
    timeout_ms: Option<u64>,
    output_limit_bytes: u64,
) -> ExecRequest {
    ExecRequest {
        id: id.to_owned(),
        command: CommandSpec {
            program: "/bin/bash".to_owned(),
            args: vec!["-c".to_owned(), script],
        },
        cwd: workspace.to_string_lossy().into_owned(),
        env: BTreeMap::from([
            ("HOME".to_owned(), std::env::var("HOME").expect("HOME")),
            ("ONLY".to_owned(), "yes".to_owned()),
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ]),
        timeout_ms,
        interactive: false,
        policy: SandboxPolicy {
            base_rights: vec![
                tree_right(Access::Read, Path::new("/")),
                tree_right(Access::Write, workspace),
            ],
            grants,
            denies: vec![],
            network: NetworkPolicy::Blocked,
            unix_socket_roots: vec![],
            output_limit_bytes,
        },
    }
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn kill_fixture(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + RELEASE_TEST_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(value) = fs::read_to_string(path) {
            return value.trim().parse().expect("fixture PID");
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("detached fixture did not report its PID");
}

fn assert_observed_detached_fixture_is_reaped(broker: &mut Broker, workspace: &Path) {
    let pid_file = workspace.join("observed-setsid.pid");
    let python = "import os,sys,time; p=os.fork(); (os.waitpid(p,0) if p else None); (time.sleep(.2),os.setsid(),open(sys.argv[1],'w').write(str(os.getpid())),time.sleep(30)) if not p else None";
    let script = format!(
        "/usr/bin/python3 -c {} {}",
        shell_quote(python),
        shell_quote(&pid_file.to_string_lossy())
    );
    let result = broker.exec(request(
        "observed-setsid-cleanup",
        workspace,
        script,
        vec![],
        Some(3_000),
        1024,
    ));
    assert!(result.timed_out);
    assert!(
        pid_file.exists(),
        "detached fixture did not start before timeout; output: {}",
        String::from_utf8_lossy(&result.output)
    );
    let pid = wait_for_pid(&pid_file);
    let alive = process_is_alive(pid);
    if alive {
        kill_fixture(pid);
    }
    assert!(
        !alive,
        "observed detached fixture PID {pid} survived terminal completion"
    );
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn canonical_missing_path(path: &Path) -> PathBuf {
    path.parent()
        .expect("test path parent")
        .canonicalize()
        .expect("canonical test path parent")
        .join(path.file_name().expect("test path name"))
}

#[test]
#[ignore = "release gate: requires an unsandboxed macOS runner"]
#[allow(clippy::too_many_lines)]
fn native_broker_release_gate() {
    let root = TempRoot::new();
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let outside = root.0.join("outside.txt");
    let mut broker = Broker::start();

    let allowed = workspace.join("allowed.txt");
    let environment = broker.exec(request(
        "environment",
        &workspace,
        format!(
            "test \"$ONLY\" = yes && test -z \"${{SECRET_TOKEN:-}}\" && printf ok > {} && printf env-ok",
            shell_quote(&allowed.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    ));
    assert_eq!(environment.code, Some(0));
    assert_eq!(environment.output, b"env-ok");
    assert_eq!(fs::read_to_string(&allowed).expect("allowed write"), "ok");

    let denied = broker.exec(request(
        "external-denied",
        &workspace,
        format!("printf bad > {}", shell_quote(&outside.to_string_lossy())),
        vec![],
        None,
        1024,
    ));
    assert_ne!(denied.code, Some(0));
    assert!(!outside.exists());
    let expected_outside = canonical_missing_path(&outside);
    assert!(
        denied.denials.iter().any(|denial| {
            denial.operation.starts_with("file-write")
                && denial.path.as_deref() == Some(expected_outside.to_string_lossy().as_ref())
        }),
        "missing exact denial for {}; got {:?}",
        expected_outside.display(),
        denied.denials
    );

    let generic_outside = root.0.join("generic-error.txt");
    let generic = broker.exec(request(
        "generic-error-denial",
        &workspace,
        format!(
            "if {{ printf bad > {}; }} 2>/dev/null; then exit 0; else printf 'service unavailable\\n'; exit 1; fi",
            shell_quote(&generic_outside.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    ));
    assert_ne!(generic.code, Some(0));
    assert_eq!(generic.output, b"service unavailable\n");
    let expected_generic = canonical_missing_path(&generic_outside);
    assert!(
        generic.denials.iter().any(|denial| {
            denial.operation.starts_with("file-write")
                && denial.path.as_deref() == Some(expected_generic.to_string_lossy().as_ref())
        }),
        "missing exact denial for {}; got {:?}",
        expected_generic.display(),
        generic.denials
    );

    let granted = broker.exec(request(
        "external-granted",
        &workspace,
        format!(
            "printf granted > {}",
            shell_quote(&outside.to_string_lossy())
        ),
        vec![file_grant(&outside)],
        None,
        1024,
    ));
    assert_eq!(granted.code, Some(0));
    assert_eq!(
        fs::read_to_string(&outside).expect("granted write"),
        "granted"
    );

    let git = workspace.join(".git");
    fs::create_dir_all(&git).expect("create git control folder");
    let git_config = git.join("config");
    let protected = broker.exec(request(
        "git-protected",
        &workspace,
        format!(
            "printf bad > {}",
            shell_quote(&git_config.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    ));
    assert_ne!(protected.code, Some(0));
    assert!(!git_config.exists());
    let approved = broker.exec(request(
        "git-approved",
        &workspace,
        format!("printf ok > {}", shell_quote(&git_config.to_string_lossy())),
        vec![tree_grant(&git)],
        None,
        1024,
    ));
    assert_eq!(approved.code, Some(0));

    let output = broker.exec(request(
        "output-cap",
        &workspace,
        "yes x | head -c 4096".to_owned(),
        vec![],
        None,
        1024,
    ));
    assert_eq!(output.code, Some(0));
    assert_eq!(output.output.len(), 1024);
    assert!(output.truncated);

    let socket = broker.exec(request(
        "socket-blocked",
        &workspace,
        "/usr/bin/python3 -c 'import socket; s=socket.socket(); s.bind((\"127.0.0.1\",0))'"
            .to_owned(),
        vec![],
        None,
        1024,
    ));
    assert_ne!(socket.code, Some(0));

    let mut loopback = request(
        "loopback-allowed",
        &workspace,
        "/usr/bin/python3 -c 'import socket; s=socket.socket(); s.bind((\"127.0.0.1\",0)); s.listen(); c=socket.create_connection(s.getsockname()); a,_=s.accept(); c.sendall(b\"ok\"); print(a.recv(2).decode(),end=\"\")'".to_owned(),
        vec![],
        None,
        1024,
    );
    loopback.policy.network = NetworkPolicy::Loopback;
    let loopback = broker.exec(loopback);
    assert_eq!(loopback.code, Some(0));
    assert!(loopback.output.ends_with(b"ok"));

    let loopback_unix_path = workspace.join("loopback.sock");
    let mut loopback_unix = request(
        "loopback-unix-allowed",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.listen()' {}",
            shell_quote(&loopback_unix_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    loopback_unix.policy.network = NetworkPolicy::Loopback;
    let loopback_unix = broker.exec(loopback_unix);
    assert_eq!(loopback_unix.code, Some(0));
    assert!(loopback_unix_path.exists());
    fs::remove_file(loopback_unix_path).expect("remove loopback Unix socket");

    let project_control = workspace.join(".pi");
    fs::create_dir_all(&project_control).expect("create project control folder");
    let project_control_socket = project_control.join("blocked.sock");
    let mut project_control_bind = request(
        "loopback-unix-project-control-blocked",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' {}",
            shell_quote(&project_control_socket.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    project_control_bind.policy.network = NetworkPolicy::Loopback;
    let project_control_bind = broker.exec(project_control_bind);
    assert_ne!(project_control_bind.code, Some(0));
    assert!(!project_control_socket.exists());

    let denied_socket_root = workspace.join("denied-sockets");
    fs::create_dir_all(&denied_socket_root).expect("create denied socket folder");
    let denied_socket_path = denied_socket_root.join("blocked.sock");
    let mut denied_socket_bind = request(
        "loopback-unix-denied-child-blocked",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' {}",
            shell_quote(&denied_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    denied_socket_bind.policy.network = NetworkPolicy::Loopback;
    denied_socket_bind.policy.denies.push(FilesystemDeny {
        access: DeniedAccess::Write,
        pattern: denied_socket_root.to_string_lossy().into_owned(),
        scope: DenyScope::Tree,
    });
    let denied_socket_bind = broker.exec(denied_socket_bind);
    assert_ne!(denied_socket_bind.code, Some(0));
    assert!(!denied_socket_path.exists());

    let glob_denied_socket_path = workspace.join("blocked.secret");
    let mut glob_denied_socket_bind = request(
        "loopback-unix-glob-denied-blocked",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' {}",
            shell_quote(&glob_denied_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    glob_denied_socket_bind.policy.network = NetworkPolicy::Loopback;
    glob_denied_socket_bind.policy.denies.push(FilesystemDeny {
        access: DeniedAccess::Write,
        pattern: format!("{}/**/*.secret", workspace.display()),
        scope: DenyScope::Glob,
    });
    let glob_denied_socket_bind = broker.exec(glob_denied_socket_bind);
    assert_ne!(glob_denied_socket_bind.code, Some(0));
    assert!(!glob_denied_socket_path.exists());

    let outside_socket_path = PathBuf::from(format!(
        "/tmp/pi-broker-{}-outside-write-root.sock",
        std::process::id()
    ));
    let _ = fs::remove_file(&outside_socket_path);
    let mut outside_socket_bind = request(
        "loopback-unix-outside-write-root-blocked",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' {}",
            shell_quote(&outside_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    outside_socket_bind.policy.network = NetworkPolicy::Loopback;
    let outside_socket_bind = broker.exec(outside_socket_bind);
    assert_ne!(outside_socket_bind.code, Some(0));
    assert!(!outside_socket_path.exists());

    let existing_socket_path = workspace.join("existing.sock");
    let existing_socket = UnixListener::bind(&existing_socket_path)
        .expect("bind existing workspace Unix socket fixture");
    let existing_socket_path = existing_socket_path
        .canonicalize()
        .expect("canonical existing workspace Unix socket fixture");
    let mut existing_socket_connect = request(
        "loopback-unix-existing-outbound-blocked",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])' {}",
            shell_quote(&existing_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    existing_socket_connect.policy.network = NetworkPolicy::Loopback;
    let existing_socket_connect = broker.exec(existing_socket_connect);
    assert_ne!(existing_socket_connect.code, Some(0));
    drop(existing_socket);
    fs::remove_file(existing_socket_path).expect("remove existing Unix socket fixture");

    let outbound = broker.exec(request(
        "outbound-blocked",
        &workspace,
        "/usr/bin/python3 -c 'import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.sendto(b\"x\",(\"127.0.0.1\",9))'".to_owned(),
        vec![],
        None,
        1024,
    ));
    assert_ne!(outbound.code, Some(0));

    let unix_socket_path = workspace.join("blocked.sock");
    let unix_socket = broker.exec(request(
        "unix-socket-blocked",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' {}",
            shell_quote(&unix_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    ));
    assert_ne!(unix_socket.code, Some(0));
    assert!(!unix_socket_path.exists());

    let short_unix_socket_path = PathBuf::from(format!(
        "/tmp/pi-broker-{}-allowed.sock",
        std::process::id()
    ));
    let _ = fs::remove_file(&short_unix_socket_path);
    let _allowed_unix_socket =
        UnixListener::bind(&short_unix_socket_path).expect("bind allowed Unix socket fixture");
    let allowed_unix_socket_path = short_unix_socket_path
        .canonicalize()
        .expect("canonical allowed Unix socket fixture");
    let mut allowed_unix_socket_request = request(
        "unix-socket-allowed",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])' {}",
            shell_quote(&allowed_unix_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    allowed_unix_socket_request.policy.unix_socket_roots =
        vec![allowed_unix_socket_path.to_string_lossy().into_owned()];
    let allowed_unix_socket = broker.exec(allowed_unix_socket_request);
    assert_eq!(allowed_unix_socket.code, Some(0));

    let sibling_unix_socket_path = PathBuf::from(format!(
        "/tmp/pi-broker-{}-sibling.sock",
        std::process::id()
    ));
    let _ = fs::remove_file(&sibling_unix_socket_path);
    let _sibling_unix_socket =
        UnixListener::bind(&sibling_unix_socket_path).expect("bind sibling Unix socket fixture");
    let sibling_unix_socket_path = sibling_unix_socket_path
        .canonicalize()
        .expect("canonical sibling Unix socket fixture");
    let mut sibling_unix_socket_request = request(
        "unix-socket-sibling-denied",
        &workspace,
        format!(
            "/usr/bin/python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])' {}",
            shell_quote(&sibling_unix_socket_path.to_string_lossy())
        ),
        vec![],
        None,
        1024,
    );
    sibling_unix_socket_request.policy.unix_socket_roots =
        vec![allowed_unix_socket_path.to_string_lossy().into_owned()];
    let sibling_unix_socket = broker.exec(sibling_unix_socket_request);
    assert_ne!(sibling_unix_socket.code, Some(0));

    fs::remove_file(short_unix_socket_path).expect("remove allowed Unix socket fixture");
    fs::remove_file(sibling_unix_socket_path).expect("remove sibling Unix socket fixture");

    let timed_out = broker.exec(request(
        "timeout",
        &workspace,
        "sleep 5".to_owned(),
        vec![],
        Some(100),
        1024,
    ));
    assert!(timed_out.timed_out);
    assert!(!timed_out.cancelled);

    let cancelled = broker.exec_and_cancel(request(
        "active-cancel",
        &workspace,
        "printf 'PI_RELEASE_READY\\n'; sleep 30".to_owned(),
        vec![],
        None,
        1024,
    ));
    assert!(cancelled.cancelled);
    assert!(!cancelled.timed_out);

    broker.send(&ClientRequest::Cancel {
        id: "already-finished".to_owned(),
    });
    let after_cancel = broker.exec(request(
        "after-idempotent-cancel",
        &workspace,
        "true".to_owned(),
        vec![],
        None,
        1024,
    ));
    assert_eq!(after_cancel.code, Some(0));

    assert_observed_detached_fixture_is_reaped(&mut broker, &workspace);

    let mut shutdown_broker = Broker::start();
    let shutdown = shutdown_broker.exec_and_shutdown(request(
        "active-shutdown",
        &workspace,
        "printf 'PI_RELEASE_READY\\n'; sleep 30".to_owned(),
        vec![],
        None,
        1024,
    ));
    assert!(shutdown.cancelled);
    assert!(!shutdown.timed_out);
    shutdown_broker.wait_for_exit();
}
