//! Best-effort descendant tracking and cleanup on macOS.
//!
//! Adapted from the Codex commit
//! `484518f28433c37d3142c49d7060bd35462ce352`,
//! `codex-rs/sandboxing/src/seatbelt_denials/pid_tracker.rs`.
//! Pi adds synchronous launch readiness, a fixed observation cap, and process
//! start-time checks. macOS does not make child enumeration and fork
//! notification atomic, so a child that deliberately detaches can still escape
//! this tracker.

#[cfg(target_os = "macos")]
mod platform {
    #![allow(unsafe_code)]

    use std::collections::{HashMap, HashSet};
    use std::io;
    use std::mem::{MaybeUninit, size_of};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::ptr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use crate::denial_collector::PidObserver;

    const STOP_IDENT: libc::uintptr_t = 1;
    const EVENTS_CAPACITY: usize = 32;
    const MAX_TRACKED_PROCESSES: usize = 4_096;
    const POLL_INTERVAL: Duration = Duration::from_millis(20);

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct ProcessIdentity {
        pid: i32,
        start_seconds: u64,
        start_microseconds: u64,
    }

    impl ProcessIdentity {
        fn read(pid: i32) -> Option<Self> {
            ProcessInfo::read(pid).map(|info| info.identity)
        }

        fn is_current(self) -> bool {
            Self::read(self.pid) == Some(self)
        }
    }

    struct ProcessInfo {
        identity: ProcessIdentity,
        parent_pid: i32,
    }

    impl ProcessInfo {
        fn read(pid: i32) -> Option<Self> {
            if pid <= 0 {
                return None;
            }
            let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            let size = i32::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
            // SAFETY: `info` points to `size` writable bytes of the exact type
            // requested by PROC_PIDTBSDINFO. We initialize it only when libc
            // reports that it wrote the whole structure.
            let written = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size,
                )
            };
            if written != size {
                return None;
            }
            // SAFETY: the exact structure size was written above.
            let info = unsafe { info.assume_init() };
            if info.pbi_pid != u32::try_from(pid).ok()? {
                return None;
            }
            Some(Self {
                identity: ProcessIdentity {
                    pid,
                    start_seconds: info.pbi_start_tvsec,
                    start_microseconds: info.pbi_start_tvusec,
                },
                parent_pid: i32::try_from(info.pbi_ppid).ok()?,
            })
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub struct ProcessGuard(ProcessIdentity);

    impl ProcessGuard {
        #[must_use]
        pub fn is_current(self) -> bool {
            self.0.is_current()
        }
    }

    struct TrackerState {
        seen: HashSet<ProcessIdentity>,
        active: HashMap<i32, ProcessIdentity>,
        observer: Option<PidObserver>,
    }

    impl TrackerState {
        fn new(observer: Option<PidObserver>) -> Self {
            Self {
                seen: HashSet::new(),
                active: HashMap::new(),
                observer,
            }
        }

        fn observe(&self, pid: i32) {
            if let Some(observer) = &self.observer {
                observer.observe(pid);
            }
        }

        fn add_pid(&mut self, kqueue: i32, pid: i32, expected_parent: ProcessIdentity) {
            let mut pending = vec![(pid, expected_parent)];
            while let Some((pid, expected_parent)) = pending.pop() {
                if !expected_parent.is_current() {
                    continue;
                }
                let Some(info) = ProcessInfo::read(pid) else {
                    continue;
                };
                if info.parent_pid != expected_parent.pid {
                    continue;
                }
                let identity = info.identity;
                if self.active.get(&pid) == Some(&identity) {
                    continue;
                }
                if !self.seen.contains(&identity) && self.seen.len() >= MAX_TRACKED_PROCESSES {
                    continue;
                }
                if watch_pid(kqueue, pid).is_err() {
                    continue;
                }
                // Reject and remove a watch if either process changed between
                // the process-info and kqueue calls.
                let valid = expected_parent.is_current()
                    && ProcessInfo::read(pid).is_some_and(|current| {
                        current.identity == identity && current.parent_pid == expected_parent.pid
                    });
                if !valid {
                    unwatch_pid(kqueue, pid);
                    continue;
                }
                self.seen.insert(identity);
                self.active.insert(pid, identity);
                self.observe(pid);
                pending.extend(
                    list_child_pids(pid)
                        .into_iter()
                        .map(|child| (child, identity)),
                );
            }
        }

        fn add_children(&mut self, kqueue: i32, parent: i32) {
            let Some(identity) = self.active.get(&parent).copied() else {
                return;
            };
            if !identity.is_current() {
                self.active.remove(&parent);
                unwatch_pid(kqueue, parent);
                return;
            }
            for child in list_child_pids(parent) {
                self.add_pid(kqueue, child, identity);
            }
        }

        fn refresh(&mut self, kqueue: i32) {
            let parents: Vec<i32> = self.active.keys().copied().collect();
            for parent in parents {
                self.add_children(kqueue, parent);
            }
        }

        fn handle(&mut self, kqueue: i32, event: &libc::kevent) -> bool {
            if event.filter == libc::EVFILT_USER && event.ident == STOP_IDENT {
                return true;
            }
            let Ok(pid) = i32::try_from(event.ident) else {
                return false;
            };
            if !self.active.contains_key(&pid) {
                return false;
            }
            if (event.flags & libc::EV_ERROR) != 0 {
                self.active.remove(&pid);
                return false;
            }
            if (event.fflags & libc::NOTE_FORK) != 0 {
                self.add_children(kqueue, pid);
            }
            if (event.fflags & libc::NOTE_EXIT) != 0 {
                self.active.remove(&pid);
            }
            false
        }
    }

    /// Tracks descendants discovered through kqueue fork events and child PID
    /// snapshots. The tracker must start before the launch barrier opens.
    pub struct PidTracker {
        root: ProcessGuard,
        stop: Arc<AtomicBool>,
        trigger: OwnedFd,
        thread: Option<JoinHandle<HashSet<ProcessIdentity>>>,
    }

    impl PidTracker {
        /// Registers the root process before returning.
        ///
        /// # Errors
        ///
        /// Returns an error when the root cannot be identified and watched or
        /// when the tracker cannot create its kqueue descriptors and thread.
        pub fn start(root_pid: i32) -> Result<Self, String> {
            Self::start_inner(root_pid, None)
        }

        /// Registers the root and reports each accepted PID to the denial collector.
        ///
        /// # Errors
        ///
        /// Returns the same setup errors as [`Self::start`].
        pub fn start_observed(root_pid: i32, observer: PidObserver) -> Result<Self, String> {
            Self::start_inner(root_pid, Some(observer))
        }

        fn start_inner(root_pid: i32, observer: Option<PidObserver>) -> Result<Self, String> {
            if root_pid <= 0 {
                return Err("cannot track an invalid root PID".to_owned());
            }
            // SAFETY: kqueue has no pointer arguments and returns a new fd.
            let raw_kqueue = unsafe { libc::kqueue() };
            if raw_kqueue < 0 {
                return Err(format!(
                    "cannot create process tracker: {}",
                    io::Error::last_os_error()
                ));
            }
            // SAFETY: `raw_kqueue` is a newly owned descriptor.
            let kqueue = unsafe { OwnedFd::from_raw_fd(raw_kqueue) };
            register_stop_event(kqueue.as_raw_fd())
                .map_err(|error| format!("cannot register process tracker stop event: {error}"))?;

            let mut state = TrackerState::new(observer);
            let root = ProcessIdentity::read(root_pid)
                .ok_or_else(|| format!("cannot identify command root PID {root_pid}"))?;
            watch_pid(kqueue.as_raw_fd(), root_pid)
                .map_err(|error| format!("cannot watch command root PID {root_pid}: {error}"))?;
            if !root.is_current() {
                return Err(format!(
                    "command root PID {root_pid} changed during tracker setup"
                ));
            }
            state.seen.insert(root);
            state.active.insert(root_pid, root);
            state.observe(root_pid);
            state.add_children(kqueue.as_raw_fd(), root_pid);

            // Keep a second descriptor so the tracking thread cannot close
            // and reuse the descriptor that `stop` uses to wake kqueue.
            // SAFETY: dup returns a new descriptor for the same kqueue.
            let raw_trigger = unsafe { libc::dup(kqueue.as_raw_fd()) };
            if raw_trigger < 0 {
                return Err(format!(
                    "cannot duplicate process tracker fd: {}",
                    io::Error::last_os_error()
                ));
            }
            // SAFETY: `raw_trigger` is a newly owned descriptor.
            let trigger = unsafe { OwnedFd::from_raw_fd(raw_trigger) };
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let thread = thread::Builder::new()
                .name("pi-sandbox-pid-tracker".to_owned())
                .spawn(move || {
                    let result = track(kqueue.as_raw_fd(), state, &thread_stop);
                    drop(kqueue);
                    result
                })
                .map_err(|error| format!("cannot start process tracker thread: {error}"))?;
            Ok(Self {
                root: ProcessGuard(root),
                stop,
                trigger,
                thread: Some(thread),
            })
        }

        #[must_use]
        pub fn root_guard(&self) -> ProcessGuard {
            self.root
        }

        fn stop_inner(&mut self) -> HashSet<ProcessIdentity> {
            self.stop.store(true, Ordering::Release);
            trigger_stop_event(self.trigger.as_raw_fd());
            self.thread
                .take()
                .and_then(|thread| thread.join().ok())
                .unwrap_or_default()
        }

        #[cfg(test)]
        fn stop(mut self) -> HashSet<ProcessIdentity> {
            self.stop_inner()
        }
    }

    impl Drop for PidTracker {
        fn drop(&mut self) {
            if self.thread.is_some() {
                let _ = self.stop_inner();
            }
        }
    }

    /// Stops tracking, then signals every observed process whose PID still has
    /// the same start time. This narrows PID-reuse risk, but macOS has no
    /// atomic pidfd-style identity-and-signal operation.
    pub fn cleanup(mut tracker: PidTracker, grace: Duration) {
        let identities = tracker.stop_inner();
        signal_current(&identities, libc::SIGTERM);
        if wait_until_gone(&identities, grace) {
            return;
        }
        signal_current(&identities, libc::SIGKILL);
        let _ = wait_until_gone(&identities, grace);
    }

    fn signal_current(identities: &HashSet<ProcessIdentity>, signal: i32) {
        for identity in identities {
            if identity.is_current() {
                // SAFETY: kill has no pointer arguments. Identity is checked
                // immediately before the call; macOS offers no atomic form.
                let _ = unsafe { libc::kill(identity.pid, signal) };
            }
        }
    }

    fn wait_until_gone(identities: &HashSet<ProcessIdentity>, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while identities.iter().any(|identity| identity.is_current()) {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(POLL_INTERVAL);
        }
        true
    }

    fn list_child_pids(parent: i32) -> Vec<i32> {
        let mut capacity = 16_usize;
        loop {
            let mut pids = vec![0_i32; capacity];
            let Ok(bytes) = i32::try_from(pids.len().saturating_mul(size_of::<i32>())) else {
                return Vec::new();
            };
            // SAFETY: `pids` is a writable array whose byte size is passed to
            // libc. The call returns a count no larger than its capacity.
            let count =
                unsafe { libc::proc_listchildpids(parent, pids.as_mut_ptr().cast(), bytes) };
            if count <= 0 {
                return Vec::new();
            }
            let Ok(returned) = usize::try_from(count) else {
                return Vec::new();
            };
            if returned < capacity || capacity >= MAX_TRACKED_PROCESSES {
                pids.truncate(returned.min(capacity));
                return pids;
            }
            capacity = capacity
                .saturating_mul(2)
                .max(returned.saturating_add(16))
                .min(MAX_TRACKED_PROCESSES);
        }
    }

    fn watch_pid(kqueue: i32, pid: i32) -> io::Result<()> {
        let event = libc::kevent {
            ident: usize::try_from(pid).unwrap_or(0),
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_FORK | libc::NOTE_EXEC | libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        submit_event(kqueue, &event)
    }

    fn unwatch_pid(kqueue: i32, pid: i32) {
        let event = libc::kevent {
            ident: usize::try_from(pid).unwrap_or(0),
            filter: libc::EVFILT_PROC,
            flags: libc::EV_DELETE,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        };
        let _ = submit_event(kqueue, &event);
    }

    fn register_stop_event(kqueue: i32) -> io::Result<()> {
        let event = libc::kevent {
            ident: STOP_IDENT,
            filter: libc::EVFILT_USER,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        };
        submit_event(kqueue, &event)
    }

    fn trigger_stop_event(kqueue: i32) {
        let event = libc::kevent {
            ident: STOP_IDENT,
            filter: libc::EVFILT_USER,
            flags: 0,
            fflags: libc::NOTE_TRIGGER,
            data: 0,
            udata: ptr::null_mut(),
        };
        let _ = submit_event(kqueue, &event);
    }

    fn submit_event(kqueue: i32, event: &libc::kevent) -> io::Result<()> {
        // SAFETY: `event` points to one initialized kevent; no output buffer is
        // supplied for this registration call.
        let result = unsafe { libc::kevent(kqueue, event, 1, ptr::null_mut(), 0, ptr::null()) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn track(
        kqueue: i32,
        mut state: TrackerState,
        stop_requested: &AtomicBool,
    ) -> HashSet<ProcessIdentity> {
        let mut events: [libc::kevent; EVENTS_CAPACITY] = std::array::from_fn(|_| libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: ptr::null_mut(),
        });
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        loop {
            if stop_requested.load(Ordering::Acquire) {
                state.refresh(kqueue);
                break;
            }
            // SAFETY: `events` is a writable fixed-size output array, timeout
            // points to an initialized value, and the kqueue descriptor remains
            // owned for this loop.
            let count = unsafe {
                libc::kevent(
                    kqueue,
                    ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    i32::try_from(EVENTS_CAPACITY).expect("event capacity fits i32"),
                    &raw const timeout,
                )
            };
            if count < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            let mut stop = false;
            for event in events.iter().take(usize::try_from(count).unwrap_or(0)) {
                stop |= state.handle(kqueue, event);
            }
            if stop {
                // One last recursive snapshot catches ordinary children that
                // forked just before the stop event. It cannot close the
                // documented fork/reparent race.
                state.refresh(kqueue);
                break;
            }
        }
        state.seen
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Command, Stdio};

        #[test]
        fn process_identity_rejects_a_different_start_time() {
            let current =
                ProcessIdentity::read(i32::try_from(std::process::id()).expect("PID fits"))
                    .expect("identify test process");
            let wrong = ProcessIdentity {
                start_microseconds: current.start_microseconds.wrapping_add(1),
                ..current
            };
            assert!(current.is_current());
            assert!(!wrong.is_current());
        }

        #[test]
        fn tracker_collects_an_ordinary_child() {
            let tracker = PidTracker::start(i32::try_from(std::process::id()).expect("PID fits"))
                .expect("start tracker");
            let mut child = Command::new("/bin/sleep")
                .arg("0.1")
                .stdin(Stdio::null())
                .spawn()
                .expect("spawn child");
            let child_pid = i32::try_from(child.id()).expect("child PID fits");
            let _ = child.wait();
            let seen = tracker.stop();
            assert!(seen.iter().any(|identity| identity.pid == child_pid));
        }

        #[test]
        fn cleanup_signals_an_observed_child() {
            let script = "read _; sleep 30 & printf '%s\\n' $!; wait";
            let mut root = Command::new("/bin/sh")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("spawn fixture root");
            let tracker = PidTracker::start(i32::try_from(root.id()).expect("root PID fits"))
                .expect("start tracker");
            root.stdin
                .take()
                .expect("fixture stdin")
                .write_all(b"go\n")
                .expect("release fixture");
            let mut line = String::new();
            BufReader::new(root.stdout.take().expect("fixture stdout"))
                .read_line(&mut line)
                .expect("read detached PID");
            let detached_pid = line.trim().parse::<i32>().expect("detached PID");

            cleanup(tracker, Duration::from_millis(250));
            let _ = root.wait();
            let alive = ProcessIdentity::read(detached_pid).is_some();
            if alive {
                // SAFETY: emergency test cleanup for the parsed fixture PID.
                let _ = unsafe { libc::kill(detached_pid, libc::SIGKILL) };
            }
            assert!(!alive, "observed child survived cleanup");
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;
    use std::time::Duration;

    use crate::denial_collector::PidObserver;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ProcessGuard {
        pid: i32,
        start_ticks: u64,
    }

    impl ProcessGuard {
        fn read(pid: i32) -> Option<Self> {
            if pid <= 0 {
                return None;
            }
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let fields = stat.rsplit_once(") ")?.1;
            let start_ticks = fields.split_whitespace().nth(19)?.parse().ok()?;
            Some(Self { pid, start_ticks })
        }

        #[must_use]
        pub fn is_current(self) -> bool {
            Self::read(self.pid) == Some(self)
        }
    }

    pub struct PidTracker(ProcessGuard);

    impl PidTracker {
        pub fn start(root_pid: i32) -> Result<Self, String> {
            ProcessGuard::read(root_pid)
                .map(Self)
                .ok_or_else(|| format!("cannot identify Linux sandbox process {root_pid}"))
        }

        pub fn start_observed(root_pid: i32, _observer: PidObserver) -> Result<Self, String> {
            Self::start(root_pid)
        }

        #[must_use]
        pub fn root_guard(&self) -> ProcessGuard {
            self.0
        }
    }

    // Bubblewrap's PID namespace is the descendant ownership boundary. When
    // its init exits, the kernel kills every remaining process in that namespace.
    pub fn cleanup(_tracker: PidTracker, _grace: Duration) {}

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn current_process_identity_is_stable() {
            let guard = ProcessGuard::read(i32::try_from(std::process::id()).expect("PID fits"))
                .expect("read current identity");
            assert!(guard.is_current());
            assert!(
                !ProcessGuard {
                    start_ticks: guard.start_ticks.wrapping_add(1),
                    ..guard
                }
                .is_current()
            );
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use std::time::Duration;

    use crate::denial_collector::PidObserver;

    #[derive(Clone, Copy, Debug)]
    pub struct ProcessGuard;

    impl ProcessGuard {
        #[must_use]
        pub fn is_current(self) -> bool {
            false
        }
    }

    pub struct PidTracker;

    impl PidTracker {
        pub fn start(_root_pid: i32) -> Result<Self, String> {
            Ok(Self)
        }

        pub fn start_observed(_root_pid: i32, _observer: PidObserver) -> Result<Self, String> {
            Ok(Self)
        }

        #[must_use]
        pub fn root_guard(&self) -> ProcessGuard {
            ProcessGuard
        }
    }

    pub fn cleanup(_tracker: PidTracker, _grace: Duration) {}
}

pub use platform::{PidTracker, ProcessGuard, cleanup};
