#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use pi_sandbox_broker::framing::{read_frame, write_frame};
use pi_sandbox_broker::protocol::{
    Access, ClientRequest, CommandSpec, ExecRequest, FilesystemRight, MissingPathBehavior,
    NetworkPolicy, PathScope, SandboxPolicy, ServerEvent,
};

const RELEASE_TEST_TIMEOUT: Duration = Duration::from_secs(5);

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
                    version: 1,
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
        let id = request.id.clone();
        self.send(&ClientRequest::Exec(request));
        let mut started = false;
        let mut output = Vec::new();
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
                    return CommandResult {
                        output,
                        code,
                        timed_out,
                        cancelled,
                        truncated: output_truncated,
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
}

impl Drop for Broker {
    fn drop(&mut self) {
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
        policy: SandboxPolicy {
            base_rights: vec![
                tree_right(Access::Read, Path::new("/")),
                tree_right(Access::Write, workspace),
            ],
            grants,
            denies: vec![],
            network: NetworkPolicy::Blocked,
            output_limit_bytes,
        },
    }
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

fn kill_fixture(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &pid.to_string()])
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

fn assert_detached_fixture_is_reaped(
    broker: &mut Broker,
    workspace: &Path,
    id: &str,
    python: &str,
) {
    let pid_file = workspace.join(format!("{id}.pid"));
    let script = format!(
        "/usr/bin/python3 -c {} {}",
        shell_quote(python),
        shell_quote(&pid_file.to_string_lossy())
    );
    let result = broker.exec(request(id, workspace, script, vec![], None, 1024));
    assert_eq!(result.code, Some(0));
    let pid = wait_for_pid(&pid_file);
    let alive = process_is_alive(pid);
    if alive {
        kill_fixture(pid);
    }
    assert!(
        !alive,
        "detached fixture PID {pid} survived terminal completion"
    );
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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

    assert_detached_fixture_is_reaped(
        &mut broker,
        &workspace,
        "setpgid-escape",
        "import os,sys,time; p=os.fork(); (os._exit(0) if p else None); os.setpgid(0,0); open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(30)",
    );
    assert_detached_fixture_is_reaped(
        &mut broker,
        &workspace,
        "setsid-double-fork",
        "import os,sys,time; p=os.fork(); (os._exit(0) if p else None); os.setsid(); p=os.fork(); (os._exit(0) if p else None); open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(30)",
    );
}
