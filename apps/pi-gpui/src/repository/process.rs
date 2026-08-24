use std::{
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use super::RepositoryError;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const TERMINATE_GRACE: Duration = Duration::from_millis(100);
const PIPE_CLOSE_GRACE: Duration = Duration::from_secs(1);
const STDERR_OUTPUT_LIMIT: usize = 256 * 1024;
const ROUTING_ENVIRONMENT: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
    "JJ_REPO",
];

#[derive(Debug)]
pub(super) struct CommandOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) stdout_truncated: bool,
    pub(super) stderr_truncated: bool,
}

#[derive(Clone, Debug)]
pub(super) struct CommandRunner {
    timeout: Duration,
    output_limit: usize,
    environment: Vec<(OsString, OsString)>,
}

impl CommandRunner {
    pub(super) fn new(
        timeout: Duration,
        output_limit: usize,
        environment: Vec<(OsString, OsString)>,
    ) -> Self {
        Self {
            timeout,
            output_limit,
            environment,
        }
    }

    pub(super) fn run(
        &self,
        program: &OsStr,
        arguments: &[OsString],
        working_directory: &Path,
    ) -> Result<CommandOutput, RepositoryError> {
        let mut command = Command::new(program);
        command
            .args(arguments)
            .current_dir(working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1");
        for name in ROUTING_ENVIRONMENT {
            command.env_remove(name);
        }
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|source| RepositoryError::Io {
            context: format!("start {}", program.to_string_lossy()),
            source,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RepositoryError::InvalidRepository(format!(
                "{} stdout was not piped",
                program.to_string_lossy()
            ))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            RepositoryError::InvalidRepository(format!(
                "{} stderr was not piped",
                program.to_string_lossy()
            ))
        })?;
        let process_group = child.id();
        let stdout_rx = drain_bounded(stdout, self.output_limit);
        let stderr_rx = drain_bounded(stderr, self.output_limit.min(STDERR_OUTPUT_LIMIT));
        let started = Instant::now();
        let status = loop {
            match child.try_wait().map_err(|source| RepositoryError::Io {
                context: format!("wait for {}", program.to_string_lossy()),
                source,
            })? {
                Some(status) => break status,
                None if started.elapsed() >= self.timeout => {
                    terminate_child(&mut child, process_group);
                    return Err(RepositoryError::CommandTimedOut {
                        program: program.to_string_lossy().into_owned(),
                        timeout: self.timeout,
                    });
                }
                None => thread::sleep(POLL_INTERVAL),
            }
        };
        let stdout = receive_drain(program, stdout_rx, process_group)?;
        let stderr = receive_drain(program, stderr_rx, process_group)?;
        Ok(CommandOutput {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        })
    }
}

#[derive(Debug)]
struct DrainedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn drain_bounded(
    mut stream: impl std::io::Read + Send + 'static,
    limit: usize,
) -> mpsc::Receiver<Result<DrainedOutput, std::io::Error>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut retained = Vec::with_capacity(limit.min(64 * 1024));
        let mut truncated = false;
        let mut buffer = [0_u8; 16 * 1024];
        let result = loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    break Ok(DrainedOutput {
                        bytes: retained,
                        truncated,
                    });
                }
                Ok(count) => {
                    let keep = limit.saturating_sub(retained.len()).min(count);
                    retained.extend_from_slice(&buffer[..keep]);
                    truncated |= keep < count;
                }
                Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
                Err(source) => break Err(source),
            }
        };
        let _send_result = sender.send(result);
    });
    receiver
}

fn receive_drain(
    program: &OsStr,
    receiver: mpsc::Receiver<Result<DrainedOutput, std::io::Error>>,
    process_group: u32,
) -> Result<DrainedOutput, RepositoryError> {
    let result = match receiver.recv_timeout(PIPE_CLOSE_GRACE) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            kill_process_group(process_group, true);
            receiver
                .recv_timeout(PIPE_CLOSE_GRACE)
                .map_err(|_| RepositoryError::ReaderStalled {
                    program: program.to_string_lossy().into_owned(),
                })?
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(RepositoryError::ReaderStalled {
                program: program.to_string_lossy().into_owned(),
            });
        }
    };
    result.map_err(|source| RepositoryError::Io {
        context: format!("read {} output", program.to_string_lossy()),
        source,
    })
}

fn terminate_child(child: &mut std::process::Child, process_group: u32) {
    kill_process_group(process_group, false);
    let deadline = Instant::now() + TERMINATE_GRACE;
    let mut reaped = false;
    while Instant::now() < deadline {
        if !reaped {
            reaped = child.try_wait().ok().flatten().is_some();
        }
        thread::sleep(POLL_INTERVAL);
    }
    kill_process_group(process_group, true);
    if !reaped {
        let _kill_result = child.kill();
        let _wait_result = child.wait();
    }
}

#[cfg(unix)]
fn kill_process_group(process_group: u32, force: bool) {
    let signal = if force { "-KILL" } else { "-TERM" };
    let group = format!("-{process_group}");
    let _status = Command::new("/bin/kill")
        .args([signal, &group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn kill_process_group(_process_group: u32, _force: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn bounds_output_while_continuing_to_drain() {
        let runner = CommandRunner::new(Duration::from_secs(2), 4, Vec::new());
        let arguments = [
            OsString::from("-c"),
            OsString::from("printf 123456789; printf abcdefghi >&2"),
        ];
        let output = runner
            .run(OsStr::new("sh"), &arguments, Path::new("/"))
            .expect("bounded command should finish");
        assert_eq!(output.stdout, b"1234");
        assert_eq!(output.stderr, b"abcd");
        assert!(output.stdout_truncated && output.stderr_truncated);
    }

    #[cfg(unix)]
    #[test]
    fn terminates_the_process_group_after_the_deadline() {
        let marker = std::env::temp_dir().join(format!(
            "pi-repository-timeout-marker-{}",
            std::process::id()
        ));
        let _remove_result = std::fs::remove_file(&marker);
        let runner = CommandRunner::new(Duration::from_millis(20), 1024, Vec::new());
        let arguments = [
            OsString::from("-c"),
            OsString::from("trap 'exit 0' TERM; (trap '' TERM; sleep 0.3; touch \"$1\") & wait"),
            OsString::from("repository-timeout-test"),
            marker.as_os_str().to_os_string(),
        ];
        let error = runner
            .run(OsStr::new("sh"), &arguments, Path::new("/"))
            .expect_err("shell should time out");
        assert!(matches!(error, RepositoryError::CommandTimedOut { .. }));
        std::thread::sleep(Duration::from_millis(400));
        assert!(!marker.exists(), "timed-out descendant was left running");
    }
}
