//! Best-effort macOS Seatbelt denial collection.
//!
//! Adapted from the Codex collector at commit
//! `484518f28433c37d3142c49d7060bd35462ce352`. Pi keeps one `/usr/bin/log`
//! stream for the broker session, bounds raw lines and retained records, and
//! associates records with one command through observed process IDs and a
//! per-command sequence window. Unified logging and descendant discovery can
//! drop or delay records, so emitted denial sets are always incomplete.

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::{HashSet, VecDeque};
    use std::io::{self, BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use regex_lite::Regex;

    use crate::protocol::Denial;

    const LOG_PATH: &str = "/usr/bin/log";
    const LOG_READY_PREFIX: &str = "Filtering the log data using ";
    const READY_TIMEOUT: Duration = Duration::from_secs(2);
    const FLUSH_GRACE: Duration = Duration::from_millis(150);
    const MAX_LINE_BYTES: usize = 64 * 1024;
    const MAX_RECENT_BYTES: usize = 256 * 1024;
    const MAX_RECENT_ITEMS: usize = 2_048;
    const MAX_COMMAND_ITEMS: usize = 128;
    const MAX_COMMAND_BYTES: usize = 64 * 1024;
    const MAX_PROCESS_CHARS: usize = 256;
    const MAX_CAPABILITY_CHARS: usize = 4_096;
    const PREDICATE: &str = r#"(((processID == 0) AND (senderImagePath CONTAINS "/Sandbox")) OR (subsystem == "com.apple.sandbox.reporting"))"#;

    #[derive(Clone, Debug)]
    struct LoggedDenial {
        sequence: u64,
        pid: i32,
        denial: Denial,
        bytes: usize,
    }

    #[derive(Debug)]
    struct ActiveCommand {
        id: String,
        start_sequence: u64,
        pids: HashSet<i32>,
    }

    #[derive(Default)]
    struct CollectorState {
        next_sequence: u64,
        recent_bytes: usize,
        recent: VecDeque<LoggedDenial>,
        active: Option<ActiveCommand>,
    }

    struct Inner {
        child: Mutex<Option<Child>>,
        stdout_thread: Mutex<Option<JoinHandle<()>>>,
        stderr_thread: Mutex<Option<JoinHandle<()>>>,
        state: Arc<Mutex<CollectorState>>,
    }

    /// Session-owned collector handle. Clones share one log process.
    #[derive(Clone)]
    pub struct DenialCollector {
        inner: Arc<Inner>,
    }

    /// Publishes process IDs observed for one command.
    #[derive(Clone)]
    pub struct PidObserver {
        state: Arc<Mutex<CollectorState>>,
        command_id: String,
    }

    impl PidObserver {
        pub fn observe(&self, pid: i32) {
            if pid <= 0 {
                return;
            }
            if let Ok(mut state) = self.state.lock()
                && let Some(active) = &mut state.active
                && active.id == self.command_id
            {
                active.pids.insert(pid);
            }
        }
    }

    impl DenialCollector {
        /// Starts `/usr/bin/log stream` and waits for its readiness line.
        ///
        /// # Errors
        ///
        /// Returns an error when the fixed log binary cannot start, its pipes
        /// cannot be opened, or it does not report readiness within two seconds.
        pub fn start() -> Result<Self, String> {
            let mut child = Command::new(LOG_PATH)
                .args(["stream", "--style", "ndjson", "--predicate", PREDICATE])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("cannot start Seatbelt denial collector: {error}"))?;
            let Some(stdout) = child.stdout.take() else {
                stop_child(&mut child);
                return Err("Seatbelt denial collector stdout is unavailable".to_owned());
            };
            let Some(stderr) = child.stderr.take() else {
                stop_child(&mut child);
                return Err("Seatbelt denial collector stderr is unavailable".to_owned());
            };
            let state = Arc::new(Mutex::new(CollectorState::default()));
            let (ready_tx, ready_rx) = mpsc::sync_channel(2);
            let stdout_state = Arc::clone(&state);
            let stdout_ready = ready_tx.clone();
            let stdout_thread = match thread::Builder::new()
                .name("pi-sandbox-denial-log".to_owned())
                .spawn(move || read_stdout(stdout, &stdout_state, &stdout_ready))
            {
                Ok(thread) => thread,
                Err(error) => {
                    stop_child(&mut child);
                    return Err(format!("cannot start denial collector reader: {error}"));
                }
            };
            let stderr_thread = match thread::Builder::new()
                .name("pi-sandbox-denial-diagnostics".to_owned())
                .spawn(move || read_stderr(stderr, &ready_tx))
            {
                Ok(thread) => thread,
                Err(error) => {
                    stop_child(&mut child);
                    let _ = stdout_thread.join();
                    return Err(format!(
                        "cannot start denial collector diagnostics: {error}"
                    ));
                }
            };

            if ready_rx.recv_timeout(READY_TIMEOUT).is_err()
                || child.try_wait().ok().flatten().is_some()
            {
                stop_child(&mut child);
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err("Seatbelt denial collector readiness timed out".to_owned());
            }

            Ok(Self {
                inner: Arc::new(Inner {
                    child: Mutex::new(Some(child)),
                    stdout_thread: Mutex::new(Some(stdout_thread)),
                    stderr_thread: Mutex::new(Some(stderr_thread)),
                    state,
                }),
            })
        }

        /// Opens one command attribution window.
        ///
        /// # Errors
        ///
        /// Returns an error if another command window is still active or the
        /// collector state lock is poisoned.
        pub fn begin(&self, id: &str) -> Result<PidObserver, String> {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| "denial collector state lock is poisoned".to_owned())?;
            if state.active.is_some() {
                return Err("denial collector already has an active command".to_owned());
            }
            state.recent.clear();
            state.recent_bytes = 0;
            let start_sequence = state.next_sequence;
            state.active = Some(ActiveCommand {
                id: id.to_owned(),
                start_sequence,
                pids: HashSet::new(),
            });
            Ok(PidObserver {
                state: Arc::clone(&self.inner.state),
                command_id: id.to_owned(),
            })
        }

        /// Closes a command window after a short log-delivery grace period.
        #[must_use]
        pub fn finish(&self, id: &str) -> Vec<Denial> {
            thread::sleep(FLUSH_GRACE);
            let Ok(mut state) = self.inner.state.lock() else {
                return Vec::new();
            };
            let Some(active) = state.active.take() else {
                return Vec::new();
            };
            if active.id != id {
                state.active = Some(active);
                return Vec::new();
            }
            let denials = collect_command_denials(&state.recent, &active);
            state.recent.clear();
            state.recent_bytes = 0;
            denials
        }

        pub fn abort(&self, id: &str) {
            if let Ok(mut state) = self.inner.state.lock()
                && state.active.as_ref().is_some_and(|active| active.id == id)
            {
                state.active = None;
                state.recent.clear();
                state.recent_bytes = 0;
            }
        }

        /// Stops and joins the persistent log process. This is idempotent.
        pub fn shutdown(&self) {
            if let Ok(mut child) = self.inner.child.lock()
                && let Some(mut child) = child.take()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Ok(mut handle) = self.inner.stdout_thread.lock()
                && let Some(handle) = handle.take()
            {
                let _ = handle.join();
            }
            if let Ok(mut handle) = self.inner.stderr_thread.lock()
                && let Some(handle) = handle.take()
            {
                let _ = handle.join();
            }
        }
    }

    fn stop_child(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn read_stdout(
        stdout: impl io::Read,
        state: &Arc<Mutex<CollectorState>>,
        ready: &mpsc::SyncSender<()>,
    ) {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(line)) = read_bounded_line(&mut reader, MAX_LINE_BYTES) {
            if line.starts_with(LOG_READY_PREFIX.as_bytes()) {
                let _ = ready.try_send(());
                continue;
            }
            let Some(parsed) = parse_log_line(&line) else {
                continue;
            };
            if let Ok(mut state) = state.lock()
                && state.active.is_some()
            {
                push_recent(&mut state, parsed);
            }
        }
    }

    fn read_stderr(stderr: impl io::Read, ready: &mpsc::SyncSender<()>) {
        let mut reader = BufReader::new(stderr);
        while let Ok(Some(line)) = read_bounded_line(&mut reader, MAX_LINE_BYTES) {
            if line.starts_with(LOG_READY_PREFIX.as_bytes()) {
                let _ = ready.try_send(());
            }
        }
    }

    fn read_bounded_line<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        let mut exceeded = false;
        loop {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                return if line.is_empty() && !exceeded {
                    Ok(None)
                } else if exceeded {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(Some(line))
                };
            }
            let end = buffer
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(buffer.len(), |index| index + 1);
            if !exceeded {
                if line.len().saturating_add(end) <= limit {
                    line.extend_from_slice(&buffer[..end]);
                } else {
                    line.clear();
                    exceeded = true;
                }
            }
            let ended = end <= buffer.len() && buffer.get(end.saturating_sub(1)) == Some(&b'\n');
            reader.consume(end);
            if ended {
                return Ok(Some(if exceeded { Vec::new() } else { line }));
            }
        }
    }

    fn parse_log_line(line: &[u8]) -> Option<(i32, Denial, usize)> {
        let json = serde_json::from_slice::<serde_json::Value>(line).ok()?;
        let message = json.get("eventMessage")?.as_str()?;
        parse_message(message)
    }

    fn parse_message(message: &str) -> Option<(i32, Denial, usize)> {
        static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let pattern = PATTERN.get_or_init(|| {
            Regex::new(r"^Sandbox:\s*(.+?)\((\d+)\)\s+deny\(.*?\)\s*(.+)$")
                .expect("fixed denial regex")
        });
        let (_, [process, pid, capability]) = pattern.captures(message)?.extract();
        if process.chars().count() > MAX_PROCESS_CHARS
            || capability.chars().count() > MAX_CAPABILITY_CHARS
        {
            return None;
        }
        let process = process.trim();
        if process.is_empty() {
            return None;
        }
        let pid = pid.trim().parse::<i32>().ok()?;
        if pid <= 0 {
            return None;
        }
        let (operation, argument) = capability
            .split_once(char::is_whitespace)
            .map_or((capability.trim(), None), |(operation, rest)| {
                (operation.trim(), Some(rest.trim()))
            });
        if operation.is_empty() || operation.len() > 128 {
            return None;
        }
        let path = (operation.starts_with("file-read") || operation.starts_with("file-write"))
            .then(|| argument.and_then(exact_absolute_path))
            .flatten();
        let denial = Denial {
            operation: operation.to_owned(),
            path,
            process: Some(process.to_owned()),
        };
        let bytes = denial.operation.len()
            + denial.path.as_ref().map_or(0, String::len)
            + denial.process.as_ref().map_or(0, String::len);
        Some((pid, denial, bytes))
    }

    fn exact_absolute_path(argument: &str) -> Option<String> {
        let value = argument.trim();
        let value = if let Some(quoted) = value.strip_prefix('"') {
            quoted.strip_suffix('"')?
        } else if let Some(quoted) = value.strip_prefix('\'') {
            quoted.strip_suffix('\'')?
        } else {
            value
        };
        if value.starts_with('/') && !value.contains('\0') && value.len() <= 4_096 {
            Some(value.to_owned())
        } else {
            None
        }
    }

    fn push_recent(state: &mut CollectorState, parsed: (i32, Denial, usize)) {
        let (pid, denial, bytes) = parsed;
        let logged = LoggedDenial {
            sequence: state.next_sequence,
            pid,
            denial,
            bytes,
        };
        state.next_sequence = state.next_sequence.wrapping_add(1);
        state.recent_bytes = state.recent_bytes.saturating_add(bytes);
        state.recent.push_back(logged);
        while state.recent.len() > MAX_RECENT_ITEMS || state.recent_bytes > MAX_RECENT_BYTES {
            let Some(removed) = state.recent.pop_front() else {
                break;
            };
            state.recent_bytes = state.recent_bytes.saturating_sub(removed.bytes);
        }
    }

    fn collect_command_denials(
        recent: &VecDeque<LoggedDenial>,
        active: &ActiveCommand,
    ) -> Vec<Denial> {
        let mut seen = HashSet::new();
        let mut bytes = 0_usize;
        let mut result = Vec::new();
        for logged in recent {
            if logged.sequence < active.start_sequence || !active.pids.contains(&logged.pid) {
                continue;
            }
            let key = (
                logged.denial.operation.clone(),
                logged.denial.path.clone(),
                logged.denial.process.clone(),
            );
            if !seen.insert(key)
                || result.len() >= MAX_COMMAND_ITEMS
                || bytes.saturating_add(logged.bytes) > MAX_COMMAND_BYTES
            {
                continue;
            }
            bytes += logged.bytes;
            result.push(logged.denial.clone());
        }
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn parses_exact_filesystem_denial() {
            let message =
                "Sandbox: issues(1234) deny(1) file-write-create /Users/test/issues/db.sqlite";
            let (pid, denial, _) = parse_message(message).expect("parse denial");
            assert_eq!(pid, 1234);
            assert_eq!(denial.operation, "file-write-create");
            assert_eq!(denial.path.as_deref(), Some("/Users/test/issues/db.sqlite"));
            assert_eq!(denial.process.as_deref(), Some("issues"));
        }

        #[test]
        fn rejects_invalid_pids_and_paths_for_non_file_operations() {
            assert!(parse_message("Sandbox: tool(0) deny(1) file-read-data /state").is_none());
            let (_, denial, _) = parse_message("Sandbox: tool(7) deny(1) network-outbound /state")
                .expect("parse non-file denial");
            assert_eq!(denial.operation, "network-outbound");
            assert_eq!(denial.path, None);
        }

        #[test]
        fn bounded_reader_discards_oversized_line_and_recovers() {
            let input = format!("{}\nok\n", "x".repeat(32));
            let mut reader = Cursor::new(input.into_bytes());
            assert_eq!(
                read_bounded_line(&mut reader, 8).expect("line"),
                Some(Vec::new())
            );
            assert_eq!(
                read_bounded_line(&mut reader, 8).expect("line"),
                Some(b"ok\n".to_vec())
            );
        }

        #[test]
        fn command_filter_uses_window_and_observed_pids() {
            let denial = Denial {
                operation: "file-write-create".to_owned(),
                path: Some("/state/db".to_owned()),
                process: Some("issues".to_owned()),
            };
            let recent = VecDeque::from([
                LoggedDenial {
                    sequence: 1,
                    pid: 7,
                    denial: denial.clone(),
                    bytes: 10,
                },
                LoggedDenial {
                    sequence: 2,
                    pid: 8,
                    denial: denial.clone(),
                    bytes: 10,
                },
            ]);
            let active = ActiveCommand {
                id: "one".to_owned(),
                start_sequence: 2,
                pids: HashSet::from([7, 8]),
            };
            assert_eq!(collect_command_denials(&recent, &active), vec![denial]);
        }

        #[test]
        fn stale_pid_observer_cannot_change_the_next_command() {
            let state = Arc::new(Mutex::new(CollectorState {
                active: Some(ActiveCommand {
                    id: "new".to_owned(),
                    start_sequence: 0,
                    pids: HashSet::new(),
                }),
                ..CollectorState::default()
            }));
            PidObserver {
                state: Arc::clone(&state),
                command_id: "old".to_owned(),
            }
            .observe(7);
            assert!(
                state
                    .lock()
                    .expect("collector state")
                    .active
                    .as_ref()
                    .expect("active command")
                    .pids
                    .is_empty()
            );
        }

        #[test]
        fn command_results_are_deduplicated_and_item_capped() {
            let mut recent = VecDeque::new();
            for sequence in 0..200_u64 {
                recent.push_back(LoggedDenial {
                    sequence,
                    pid: 7,
                    denial: Denial {
                        operation: "file-write-create".to_owned(),
                        path: Some(format!("/state/{sequence}")),
                        process: Some("tool".to_owned()),
                    },
                    bytes: 24,
                });
            }
            recent.push_back(recent[0].clone());
            let active = ActiveCommand {
                id: "one".to_owned(),
                start_sequence: 0,
                pids: HashSet::from([7]),
            };
            let result = collect_command_denials(&recent, &active);
            assert_eq!(result.len(), MAX_COMMAND_ITEMS);
            assert_eq!(result[0].path.as_deref(), Some("/state/0"));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use crate::protocol::Denial;

    #[derive(Clone)]
    pub struct DenialCollector;

    #[derive(Clone)]
    pub struct PidObserver;

    impl PidObserver {
        pub fn observe(&self, _pid: i32) {}
    }

    impl DenialCollector {
        pub fn start() -> Result<Self, String> {
            Err("Seatbelt denial collection requires macOS".to_owned())
        }

        pub fn begin(&self, _id: &str) -> Result<PidObserver, String> {
            Ok(PidObserver)
        }

        pub fn finish(&self, _id: &str) -> Vec<Denial> {
            Vec::new()
        }

        pub fn abort(&self, _id: &str) {}

        pub fn shutdown(&self) {}
    }
}

pub use platform::{DenialCollector, PidObserver};
