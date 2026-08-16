use std::io::{BufWriter, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::denial_collector::DenialCollector;
use crate::framing::write_frame;
use crate::pid_tracker::{PidTracker, ProcessGuard, cleanup as cleanup_tracked_processes};
use crate::protocol::{ErrorCode, ServerEvent};
#[cfg(target_os = "macos")]
use crate::seatbelt::{SANDBOX_EXEC, build_args_with_network};
use crate::validation::ValidatedExec;
use crate::validation::ValidatedNetworkPolicy;

const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATE_GRACE: Duration = Duration::from_millis(250);
const LAUNCH_SCRIPT: &str = "IFS= read -r _ || exit 125; exec \"$@\"";
const LAUNCH_PENDING: u8 = 0;
const LAUNCH_RELEASED: u8 = 1;
const LAUNCH_CANCELLED: u8 = 2;

type SharedWriter = Arc<Mutex<BufWriter<std::io::Stdout>>>;
type SharedState = Arc<(Mutex<RuntimeState>, Condvar)>;

struct PreparedResources {
    _files: Vec<std::fs::File>,
    #[cfg(target_os = "linux")]
    _synthetic_directories: Vec<crate::linux::SyntheticDirectory>,
}

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
    root: Mutex<Option<ProcessGuard>>,
    stdin: Mutex<Option<ChildStdin>>,
    launch: AtomicU8,
}

#[derive(Default)]
struct RuntimeState {
    active: Option<Arc<CommandControl>>,
}

pub struct Runtime {
    state: SharedState,
    writer: SharedWriter,
    denial_collector: Option<DenialCollector>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Runtime {
    #[must_use]
    pub fn new(stdout: std::io::Stdout) -> Self {
        Self::new_with_collector(stdout, None)
    }

    #[must_use]
    pub fn new_with_collector(
        stdout: std::io::Stdout,
        denial_collector: Option<DenialCollector>,
    ) -> Self {
        Self {
            state: Arc::new((Mutex::new(RuntimeState::default()), Condvar::new())),
            writer: Arc::new(Mutex::new(BufWriter::new(stdout))),
            denial_collector,
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

    /// Starts one command. Protocol v4 permits no parallel command per broker.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate IDs, a busy broker, or a poisoned state lock.
    pub fn start(&self, request: ValidatedExec) -> Result<(), (ErrorCode, String)> {
        let control = Arc::new(CommandControl {
            id: request.id.clone(),
            cancel: AtomicBool::new(false),
            pid: AtomicI32::new(0),
            root: Mutex::new(None),
            stdin: Mutex::new(None),
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
                        "protocol v4 permits one active command per broker".to_owned(),
                    )
                });
            }
            state.active = Some(Arc::clone(&control));
        }

        let state = Arc::clone(&self.state);
        let writer = Arc::clone(&self.writer);
        let denial_collector = self.denial_collector.clone();
        thread::spawn(move || {
            run_command(
                &request,
                &control,
                &writer,
                &state,
                denial_collector.as_ref(),
            );
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

    /// Writes bytes to an active interactive command.
    ///
    /// # Errors
    ///
    /// Returns an error when the command is absent, its stdin is closed, the
    /// state lock is poisoned, or the pipe write fails.
    pub fn write_stdin(&self, id: &str, data: &[u8]) -> Result<(), (ErrorCode, String)> {
        let control = {
            let (state, _) = &*self.state;
            let state = state.lock().map_err(|_| {
                (
                    ErrorCode::ProtocolError,
                    "runtime state lock is poisoned".to_owned(),
                )
            })?;
            state
                .active
                .clone()
                .filter(|active| active.id == id)
                .ok_or_else(|| (ErrorCode::NotFound, format!("command is not active: {id}")))?
        };
        let mut stdin = control.stdin.lock().map_err(|_| {
            (
                ErrorCode::ProtocolError,
                "command stdin lock is poisoned".to_owned(),
            )
        })?;
        let stdin = stdin.as_mut().ok_or_else(|| {
            (
                ErrorCode::InvalidRequest,
                format!("command stdin is not open: {id}"),
            )
        })?;
        stdin.write_all(data).map_err(|error| {
            (
                ErrorCode::InvalidRequest,
                format!("cannot write command stdin: {error}"),
            )
        })
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
        if let Some(collector) = &self.denial_collector {
            collector.shutdown();
        }
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
    denial_collector: Option<&DenialCollector>,
) {
    let command = launch_command(request);
    #[cfg(target_os = "macos")]
    let prepared = crate::conceal::wrap_command(&request.program, &request.args, &request.denies)
        .and_then(|concealed| {
            build_args_with_network(
                concealed.as_ref().unwrap_or(&command),
                &request.cwd,
                &request.rights,
                &request.denies,
                &request.unix_socket_roots,
                &request.network,
            )
        })
        .map(|args| {
            (
                SANDBOX_EXEC,
                args,
                "seatbelt-broker",
                PreparedResources { _files: Vec::new() },
            )
        });
    #[cfg(target_os = "linux")]
    let prepared = crate::linux::prepare(request, &command).map(|launch| {
        (
            launch.program,
            launch.args,
            "bubblewrap-broker",
            PreparedResources {
                _files: launch.resources,
                _synthetic_directories: launch.synthetic_directories,
            },
        )
    });
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let prepared: Result<(&str, Vec<String>, &str, PreparedResources), String> =
        Err("native sandbox execution is unsupported on this platform".to_owned());
    let (program, args, marker, resources) = match prepared {
        Ok(prepared) => prepared,
        Err(message) => {
            send_terminal_error(writer, state, control, ErrorCode::PolicyRejected, message);
            return;
        }
    };
    let mut environment = request.env.clone();
    strip_proxy_environment(&mut environment);
    if let ValidatedNetworkPolicy::Proxy {
        tcp_port,
        allow_local_binding,
        ..
    } = &request.network
    {
        #[cfg(target_os = "linux")]
        let proxy_port = {
            let _ = tcp_port;
            crate::linux::PROXY_LOOPBACK_PORT
        };
        #[cfg(not(target_os = "linux"))]
        let proxy_port = *tcp_port;
        let http_proxy = format!("http://127.0.0.1:{proxy_port}");
        let socks_proxy = format!("socks5h://127.0.0.1:{proxy_port}");
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
            "NPM_CONFIG_PROXY",
            "NPM_CONFIG_HTTP_PROXY",
            "NPM_CONFIG_HTTPS_PROXY",
        ] {
            environment.insert(name.to_owned(), http_proxy.clone());
        }
        for name in ["ALL_PROXY", "all_proxy"] {
            environment.insert(name.to_owned(), socks_proxy.clone());
        }
        let no_proxy = if *allow_local_binding {
            "localhost,127.0.0.1,::1"
        } else {
            ""
        };
        environment.insert("NO_PROXY".to_owned(), no_proxy.to_owned());
        environment.insert("no_proxy".to_owned(), no_proxy.to_owned());
    }
    let mut process = Command::new(program);
    process
        .args(args)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(environment)
        .env("IN_SANDBOX", "1")
        .env("PI_SANDBOX", marker)
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
                format!("cannot start {program}: {error}"),
            );
            return;
        }
    };
    // Keep inherited descriptors open and synthetic mount targets registered
    // until Bubblewrap and every process in its PID namespace have exited.
    let _resources = resources;
    let pid = i32::try_from(child.id()).unwrap_or(i32::MAX);
    control.pid.store(pid, Ordering::Release);
    let observer = match denial_collector.map(|collector| collector.begin(&request.id)) {
        Some(Ok(observer)) => Some(observer),
        Some(Err(message)) => {
            terminate_child(&mut child, control);
            clear_process(control);
            send_terminal_error(
                writer,
                state,
                control,
                ErrorCode::CommandStartFailed,
                message,
            );
            return;
        }
        None => None,
    };
    // The fixed launch wrapper is still blocked on stdin here. Register the
    // root with the best-effort descendant tracker before user code can fork.
    let tracker_result = match observer {
        Some(observer) => PidTracker::start_observed(pid, observer),
        None => PidTracker::start(pid),
    };
    let tracker = match tracker_result {
        Ok(tracker) => tracker,
        Err(message) => {
            terminate_child(&mut child, control);
            abort_denials(denial_collector, &request.id);
            clear_process(control);
            send_terminal_error(
                writer,
                state,
                control,
                ErrorCode::CommandStartFailed,
                message,
            );
            return;
        }
    };
    let Ok(mut root) = control.root.lock() else {
        terminate_child(&mut child, control);
        cleanup_tracked_processes(tracker, TERMINATE_GRACE);
        abort_denials(denial_collector, &request.id);
        clear_process(control);
        send_terminal_error(
            writer,
            state,
            control,
            ErrorCode::CommandStartFailed,
            "process identity lock is poisoned".to_owned(),
        );
        return;
    };
    *root = Some(tracker.root_guard());
    drop(root);
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
        cleanup_tracked_processes(tracker, TERMINATE_GRACE);
        abort_denials(denial_collector, &request.id);
        clear_process(control);
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
        cleanup_tracked_processes(tracker, TERMINATE_GRACE);
        abort_denials(denial_collector, &request.id);
        clear_process(control);
        return;
    }
    // The fixed launch wrapper waits on stdin. Release it only after the PID and
    // command ID are registered, then close the pipe so user code gets EOF.
    if let Some(mut barrier) = child.stdin.take() {
        let _ = barrier.write_all(b"go\n");
        if request.interactive
            && let Ok(mut stdin) = control.stdin.lock()
        {
            *stdin = Some(barrier);
        }
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
    cancelled |= control.cancel.load(Ordering::Acquire);
    cleanup_group(control);
    cleanup_tracked_processes(tracker, TERMINATE_GRACE);
    if let Some(reader) = stdout {
        finish_stream_reader(reader);
    }
    if let Some(reader) = stderr {
        finish_stream_reader(reader);
    }
    if let Some(collector) = denial_collector {
        let _ = send_event(
            writer,
            &ServerEvent::Denials {
                id: request.id.clone(),
                items: collector.finish(&request.id),
                complete: false,
            },
        );
    }
    clear_process(control);
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

fn strip_proxy_environment(environment: &mut std::collections::BTreeMap<String, String>) {
    const PROXY_NAMES: [&str; 12] = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "NPM_CONFIG_PROXY",
        "NPM_CONFIG_HTTP_PROXY",
        "NPM_CONFIG_HTTPS_PROXY",
        "GIT_PROXY_COMMAND",
    ];
    for name in PROXY_NAMES {
        environment.remove(name);
    }
}

fn abort_denials(collector: Option<&DenialCollector>, id: &str) {
    if let Some(collector) = collector {
        collector.abort(id);
    }
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

fn root_is_current(control: &CommandControl) -> bool {
    control
        .root
        .lock()
        .ok()
        .and_then(|root| *root)
        .is_some_and(ProcessGuard::is_current)
}

fn signal_group(control: &CommandControl, signal: Signal) {
    let pid = control.pid.load(Ordering::Acquire);
    if pid > 0 && root_is_current(control) {
        let _ = killpg(Pid::from_raw(pid), signal);
    }
}

fn group_exists(control: &CommandControl) -> bool {
    let pid = control.pid.load(Ordering::Acquire);
    pid > 0 && root_is_current(control) && killpg(Pid::from_raw(pid), None).is_ok()
}

fn clear_process(control: &CommandControl) {
    if let Ok(mut stdin) = control.stdin.lock() {
        *stdin = None;
    }
    if let Ok(mut root) = control.root.lock() {
        *root = None;
    }
    control.pid.store(0, Ordering::Release);
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
