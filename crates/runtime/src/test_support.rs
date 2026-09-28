use std::{path::Path, sync::Arc};

use farcaster_agent_protocol::extensions::SlashCommand;
use farcaster_agents::WorkerSnapshot;
use farcaster_storage::{SharedStateStore, StateStore};

use crate::{RuntimeHost, RuntimeMetric, RuntimeTimer, ScheduledWake, WorkerActivity};

struct TestHost {
    store: SharedStateStore,
    _directory: Option<tempfile::TempDir>,
    workers: Option<WorkerTestState>,
    idle_metrics: Arc<IdleTestMetrics>,
}

#[derive(Default)]
pub(crate) struct IdleTestMetrics {
    pub scheduled: std::sync::Mutex<Vec<std::time::Instant>>,
    pub checks: std::sync::atomic::AtomicUsize,
    pub snapshots: std::sync::atomic::AtomicUsize,
    pub inbox_checks: std::sync::atomic::AtomicUsize,
}

struct TestWorkerActivity(WorkerTestState);
impl WorkerActivity for TestWorkerActivity {
    fn revision(&self) -> u64 {
        self.0.lock().unwrap().0
    }
}
struct TestWake;
impl ScheduledWake for TestWake {}

pub(crate) type WorkerTestState = Arc<std::sync::Mutex<(u64, Vec<WorkerSnapshot>, bool)>>;

#[cfg(test)]
pub(crate) fn host_with_idle_metrics(
    workers: WorkerTestState,
) -> (Arc<dyn RuntimeHost>, Arc<IdleTestMetrics>) {
    let directory = tempfile::tempdir().expect("test directory");
    let metrics = Arc::new(IdleTestMetrics::default());
    (
        Arc::new(TestHost {
            store: SharedStateStore::new(
                StateStore::open_at(&directory.path().join("state.sqlite3")).unwrap(),
            ),
            _directory: Some(directory),
            workers: Some(workers),
            idle_metrics: metrics.clone(),
        }),
        metrics,
    )
}

/// Unit hosts own separate stores, even when the test process sets FARCASTER_DATA_DIR.
pub fn host() -> Arc<dyn RuntimeHost> {
    let directory = tempfile::tempdir().expect("create test state directory");
    let path = directory.path().join("state.sqlite3");
    host_with_path(&path, Some(directory))
}

/// Subprocess fixtures opt into sharing the same on-disk store across restarts.
pub fn host_at(path: &Path) -> Arc<dyn RuntimeHost> {
    host_with_path(path, None)
}

fn host_with_path(path: &Path, directory: Option<tempfile::TempDir>) -> Arc<dyn RuntimeHost> {
    Arc::new(TestHost {
        store: SharedStateStore::new(StateStore::open_at(path).expect("open test state")),
        _directory: directory,
        workers: None,
        idle_metrics: Arc::default(),
    })
}

impl RuntimeHost for TestHost {
    fn state_store(&self) -> Result<SharedStateStore, String> {
        Ok(self.store.clone())
    }

    fn set_worker_app_proxy(&self, proxy: Option<String>) -> Result<(), String> {
        farcaster_mcp_server::set_worker_app_proxy(proxy)
    }

    fn stop_session_family_workers(
        &self,
        project: &Path,
        sessions: &[farcaster_sessions::SessionKey],
    ) -> Result<usize, String> {
        farcaster_mcp_server::stop_session_family_workers(project, sessions)
    }

    fn finish_session_family_worker_stop(
        &self,
        project: &Path,
        sessions: &[farcaster_sessions::SessionKey],
    ) -> Result<(), String> {
        farcaster_mcp_server::finish_session_family_worker_stop(project, sessions)
    }

    fn worker_snapshots(&self) -> Result<Vec<WorkerSnapshot>, String> {
        self.idle_metrics
            .snapshots
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(workers) = &self.workers {
            let mut workers = workers.lock().unwrap();
            if workers.2 {
                workers.0 += 1;
                workers.2 = false;
            }
            return Ok(workers.1.clone());
        }
        farcaster_mcp_server::worker_snapshots()
    }

    fn schedule_wake(
        &self,
        deadline: std::time::Instant,
        _wake: std::thread::Thread,
    ) -> Box<dyn ScheduledWake> {
        self.idle_metrics.scheduled.lock().unwrap().push(deadline);
        Box::new(TestWake)
    }

    fn subscribe_worker_activity(
        &self,
        project: &Path,
        wake: std::thread::Thread,
    ) -> Box<dyn WorkerActivity> {
        match &self.workers {
            Some(workers) => Box::new(TestWorkerActivity(workers.clone())),
            None => Box::new(farcaster_agents::subscribe_worker_activity(project, wake)),
        }
    }

    fn worker_inboxes_idle(&self, project: &Path) -> bool {
        self.idle_metrics
            .inbox_checks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        farcaster_agents::CallerRegistry::shared().child_inboxes_idle(project)
    }

    fn contains_invocation(&self, _input: &str, _commands: &[SlashCommand]) -> bool {
        false
    }

    fn count_snapshot(&self) {}

    fn count_stream_event(&self, _coalesced: bool) {}

    fn timer(&self, metric: RuntimeMetric) -> Box<dyn RuntimeTimer> {
        if matches!(metric, RuntimeMetric::IdleRetirement) {
            self.idle_metrics
                .checks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Box::new(NoopTimer)
    }
}

struct NoopTimer;

impl RuntimeTimer for NoopTimer {}

#[cfg(test)]
#[path = "test_support_tests.rs"]
mod tests;
