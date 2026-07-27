use std::io::{BufWriter, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::framing::write_frame;
use crate::protocol::{ErrorCode, ServerEvent};
use crate::seatbelt::{SANDBOX_EXEC, build_args};
use crate::validation::ValidatedExec;

const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATE_GRACE: Duration = Duration::from_millis(250);
const LAUNCH_SCRIPT: &str = "IFS= read -r _ || exit 125; exec \"$@\"";
const LAUNCH_PENDING: u8 = 0;
const LAUNCH_RELEASED: u8 = 1;
const LAUNCH_CANCELLED: u8 = 2;

type SharedWriter = Arc<Mutex<BufWriter<std::io::Stdout>>>;
type SharedState = Arc<(Mutex<RuntimeState>, Condvar)>;

struct StreamReader {
    thread: thread::JoinHandle<()>,
    done: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
}

#[derive(Debug)]
struct CommandControl {
    id: String,
    cancel: AtomicBool,
    pid: AtomicI32,
    launch: AtomicU8,
}

#[derive(Default)]
struct RuntimeState {
    active: Option<Arc<CommandControl>>,
}

pub struct Runtime {
    state: SharedState,
    writer: SharedWriter,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Runtime {
    #[must_use]
    pub fn new(stdout: std::io::Stdout) -> Self {
        Self {
            state: Arc::new((Mutex::new(RuntimeState::default()), Condvar::new())),
            writer: Arc::new(Mutex::new(BufWriter::new(stdout))),
        }
    }

    /// Writes one event to the private broker channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel lock is poisoned or the frame cannot be written.
    pub fn send(&self, event: &ServerEvent) -> Result<(), String> {
        send_event(&self.writer, event)
    }

    /// Starts one command. Protocol v1 permits no parallel command.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate IDs, a busy broker, or a poisoned state lock.
    pub fn start(&self, request: ValidatedExec) -> Result<(), (ErrorCode, String)> {
        let control = Arc::new(CommandControl {
            id: request.id.clone(),
            cancel: AtomicBool::new(false),
            pid: AtomicI32::new(0),
            launch: AtomicU8::new(LAUNCH_PENDING),
        });
        {
            let (state, _) = &*self.state;
            let mut state = state.lock().map_err(|_| {
                (
                    ErrorCode::ProtocolError,
                    "runtime state lock is poisoned".to_owned(),
                )
            })?;
            if let Some(active) = &state.active {
                return Err(if active.id == request.id {
                    (
                        ErrorCode::DuplicateCommandId,
                        format!("duplicate active command ID: {}", request.id),
                    )
                } else {
                    (
                        ErrorCode::InvalidRequest,
                        "protocol v1 permits one active command".to_owned(),
                    )
                });
            }
            state.active = Some(Arc::clone(&control));
        }

        let state = Arc::clone(&self.state);
        let writer = Arc::clone(&self.writer);
        thread::spawn(move || {
            run_command(&request, &control, &writer, &state);
            clear_active(&state, &control);
        });
        Ok(())
    }

    /// Requests cancellation for the active command with this ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the ID is not active or the state lock is poisoned.
    pub fn cancel(&self, id: &str) -> Result<(), (ErrorCode, String)> {
        let control = {
            let (state, _) = &*self.state;
            let state = state.lock().map_err(|_| {
                (
                    ErrorCode::ProtocolError,
                    "runtime state lock is poisoned".to_owned(),
                )
            })?;
            let Some(active) = state.active.clone() else {
                // Cancellation is idempotent. The terminal event may have
                // crossed the request on the private protocol pipe.
                return Ok(());
            };
            active
        };
        if control.id != id {
            return Err((ErrorCode::NotFound, format!("command is not active: {id}")));
        }
        control.cancel.store(true, Ordering::Release);
        let _ = control.launch.compare_exchange(
            LAUNCH_PENDING,
            LAUNCH_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        signal_group(&control, Signal::SIGTERM);
        Ok(())
    }

    pub fn shutdown(&self) {
        let control = {
            let (state, _) = &*self.state;
            state.lock().ok().and_then(|state| state.active.clone())
        };
        if let Some(control) = control {
            control.cancel.store(true, Ordering::Release);
            let _ = control.launch.compare_exchange(
                LAUNCH_PENDING,
                LAUNCH_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            signal_group(&control, Signal::SIGTERM);
        }
        self.wait_for_idle(Duration::from_secs(3));
    }

    fn wait_for_idle(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let (state, changed) = &*self.state;
        let Ok(mut state) = state.lock() else {
            return;
        };
        while state.active.is_some() {
            let now = Instant::now();
            if now >= deadline {
                if let Some(control) = &state.active {
                    signal_group(control, Signal::SIGKILL);
                }
                return;
            }
            let Ok((next, _)) = changed.wait_timeout(state, deadline - now) else {
                return;
            };
            state = next;
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run_command(
    request: &ValidatedExec,
    control: &Arc<CommandControl>,
    writer: &SharedWriter,
    state: &SharedState,
) {
    let command = launch_command(request);
    let args = match build_args(&command, &request.rights, &request.denies) {
        Ok(args) => args,
        Err(message) => {
            send_terminal_error(writer, state, control, ErrorCode::PolicyRejected, message);
            return;
        }
    };
    let mut process = Command::new(SANDBOX_EXEC);
    process
        .args(args)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(&request.env)
        .env("IN_SANDBOX", "1")
        .env("PI_SANDBOX", "seatbelt-broker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            send_terminal_error(
                writer,
                state,
                control,
                ErrorCode::CommandStartFailed,
                format!("cannot start {SANDBOX_EXEC}: {error}"),
            );
            return;
        }
    };
    let pid = i32::try_from(child.id()).unwrap_or(i32::MAX);
    control.pid.store(pid, Ordering::Release);
    if control
        .launch
        .compare_exchange(
            LAUNCH_PENDING,
            LAUNCH_RELEASED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        terminate_child(&mut child, control);
        control.pid.store(0, Ordering::Release);
        send_terminal_error(
            writer,
            state,
            control,
            ErrorCode::Cancelled,
            "command was cancelled before launch".to_owned(),
        );
        return;
    }
    if send_event(
        writer,
        &ServerEvent::Started {
            id: request.id.clone(),
            pid: child.id(),
        },
    )
    .is_err()
    {
        terminate_child(&mut child, control);
        control.pid.store(0, Ordering::Release);
        return;
    }
    // The fixed shell wrapper waits on stdin. Release it only after the PID and
    // command ID are registered, then close the pipe so user code gets EOF.
    if let Some(mut barrier) = child.stdin.take() {
        let _ = barrier.write_all(b"go\n");
    }

    let output_used = Arc::new(AtomicU64::new(0));
    let output_truncated = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().map(|stream| {
        spawn_stream_reader(
            stream,
            request.id.clone(),
            true,
            request.output_limit_bytes,
            Arc::clone(&output_used),
            Arc::clone(&output_truncated),
            Arc::clone(writer),
        )
    });
    let stderr = child.stderr.take().map(|stream| {
        spawn_stream_reader(
            stream,
            request.id.clone(),
            false,
            request.output_limit_bytes,
            Arc::clone(&output_used),
            Arc::clone(&output_truncated),
            Arc::clone(writer),
        )
    });

    let start = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let mut termination_started = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => {
                signal_group(control, Signal::SIGKILL);
                break child.wait().ok();
            }
        }
        if request
            .timeout_ms
            .is_some_and(|timeout| start.elapsed() >= Duration::from_millis(timeout))
        {
            timed_out = true;
        }
        cancelled = control.cancel.load(Ordering::Acquire);
        if (timed_out || cancelled) && termination_started.is_none() {
            signal_group(control, Signal::SIGTERM);
            termination_started = Some(Instant::now());
        }
        if termination_started.is_some_and(|at| at.elapsed() >= TERMINATE_GRACE) {
            signal_group(control, Signal::SIGKILL);
        }
        thread::sleep(POLL_INTERVAL);
    };
    cleanup_group(control);
    if let Some(reader) = stdout {
        finish_stream_reader(reader);
    }
    if let Some(reader) = stderr {
        finish_stream_reader(reader);
    }
    control.pid.store(0, Ordering::Release);
    let _ = send_event(
        writer,
        &ServerEvent::Exit {
            id: request.id.clone(),
            code: status.as_ref().and_then(std::process::ExitStatus::code),
            signal: status.as_ref().and_then(ExitStatusExt::signal),
            timed_out,
            cancelled,
            output_truncated: output_truncated.load(Ordering::Acquire),
        },
    );
}

fn launch_command(request: &ValidatedExec) -> Vec<String> {
    let mut command = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        LAUNCH_SCRIPT.to_owned(),
        "pi-sandbox-launch".to_owned(),
        request.program.to_string_lossy().into_owned(),
    ];
    command.extend(request.args.clone());
    command
}

#[allow(clippy::too_many_arguments)]
fn spawn_stream_reader<R>(
    mut stream: R,
    id: String,
    stdout: bool,
    limit: u64,
    used: Arc<AtomicU64>,
    truncated: Arc<AtomicBool>,
    writer: SharedWriter,
) -> StreamReader
where
    R: Read + AsFd + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (done_tx, done) = mpsc::channel();
    let thread = thread::spawn(move || {
        let mut sequence = 0_u64;
        let mut buffer = [0_u8; OUTPUT_CHUNK_BYTES];
        while !thread_stop.load(Ordering::Acquire) {
            let ready = {
                let mut descriptors = [PollFd::new(
                    stream.as_fd(),
                    PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR,
                )];
                if poll(&mut descriptors, PollTimeout::from(50_u16)).is_err() {
                    break;
                }
                descriptors[0].revents().unwrap_or_else(PollFlags::empty)
            };
            if !ready.intersects(PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR) {
                continue;
            }
            let Ok(read) = stream.read(&mut buffer) else {
                break;
            };
            if read == 0 {
                break;
            }
            let allowed = usize::try_from(claim_output(
                &used,
                limit,
                u64::try_from(read).expect("output chunk size fits u64"),
            ))
            .expect("claimed output is no larger than the input buffer");
            if allowed < read {
                truncated.store(true, Ordering::Release);
            }
            if allowed == 0 {
                continue;
            }
            let data_base64 = BASE64.encode(&buffer[..allowed]);
            let event = if stdout {
                ServerEvent::Stdout {
                    id: id.clone(),
                    sequence,
                    data_base64,
                }
            } else {
                ServerEvent::Stderr {
                    id: id.clone(),
                    sequence,
                    data_base64,
                }
            };
            if send_event(&writer, &event).is_err() {
                break;
            }
            sequence += 1;
        }
        let _ = done_tx.send(());
    });
    StreamReader { thread, done, stop }
}

fn finish_stream_reader(reader: StreamReader) {
    if reader.done.recv_timeout(TERMINATE_GRACE).is_err() {
        reader.stop.store(true, Ordering::Release);
    }
    let _ = reader.thread.join();
}

fn claim_output(used: &AtomicU64, limit: u64, requested: u64) -> u64 {
    let mut current = used.load(Ordering::Acquire);
    loop {
        let allowed = requested.min(limit.saturating_sub(current));
        if allowed == 0 {
            return 0;
        }
        match used.compare_exchange_weak(
            current,
            current + allowed,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return allowed,
            Err(updated) => current = updated,
        }
    }
}

fn signal_group(control: &CommandControl, signal: Signal) {
    let pid = control.pid.load(Ordering::Acquire);
    if pid > 0 {
        let _ = killpg(Pid::from_raw(pid), signal);
    }
}

fn group_exists(control: &CommandControl) -> bool {
    let pid = control.pid.load(Ordering::Acquire);
    pid > 0 && killpg(Pid::from_raw(pid), None).is_ok()
}

fn cleanup_group(control: &CommandControl) {
    if !group_exists(control) {
        return;
    }
    signal_group(control, Signal::SIGTERM);
    if wait_for_group_exit(control, TERMINATE_GRACE) {
        return;
    }
    signal_group(control, Signal::SIGKILL);
    let _ = wait_for_group_exit(control, TERMINATE_GRACE);
}

fn wait_for_group_exit(control: &CommandControl, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while group_exists(control) && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
    }
    !group_exists(control)
}

fn terminate_child(child: &mut std::process::Child, control: &CommandControl) {
    drop(child.stdin.take());
    signal_group(control, Signal::SIGTERM);
    let deadline = Instant::now() + TERMINATE_GRACE;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            cleanup_group(control);
            return;
        }
        thread::sleep(POLL_INTERVAL);
    }
    signal_group(control, Signal::SIGKILL);
    let _ = child.wait();
    cleanup_group(control);
}

fn clear_active(state: &SharedState, control: &Arc<CommandControl>) {
    let (state, changed) = &**state;
    if let Ok(mut state) = state.lock() {
        if state
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, control))
        {
            state.active = None;
        }
        changed.notify_all();
    }
}

fn send_terminal_error(
    writer: &SharedWriter,
    _state: &SharedState,
    control: &Arc<CommandControl>,
    code: ErrorCode,
    message: String,
) {
    let _ = send_event(
        writer,
        &ServerEvent::Error {
            id: Some(control.id.clone()),
            code,
            message,
        },
    );
}

fn send_event(writer: &SharedWriter, event: &ServerEvent) -> Result<(), String> {
    let mut writer = writer
        .lock()
        .map_err(|_| "broker output lock is poisoned".to_owned())?;
    write_frame(&mut *writer, event).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_idempotent_after_a_command_is_gone() {
        let runtime = Runtime::new(std::io::stdout());
        assert!(runtime.cancel("already-finished").is_ok());
    }

    #[test]
    fn output_claim_never_exceeds_limit() {
        let used = AtomicU64::new(0);
        assert_eq!(claim_output(&used, 5, 3), 3);
        assert_eq!(claim_output(&used, 5, 4), 2);
        assert_eq!(claim_output(&used, 5, 1), 0);
        assert_eq!(used.load(Ordering::Acquire), 5);
    }

    #[test]
    fn launch_barrier_eof_never_runs_user_code() {
        let root = std::env::temp_dir().join(format!(
            "pi-launch-barrier-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create test root");
        let marker = root.join("ran");
        let status = Command::new("/bin/sh")
            .args([
                "-c",
                LAUNCH_SCRIPT,
                "pi-sandbox-launch",
                "/bin/sh",
                "-c",
                &format!("touch '{}'", marker.display()),
            ])
            .stdin(Stdio::null())
            .status()
            .expect("run launch wrapper");
        assert_eq!(status.code(), Some(125));
        assert!(!marker.exists());
    }
}
