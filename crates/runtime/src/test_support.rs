use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use farcaster_agent_protocol::extensions::SlashCommand;
use farcaster_agents::{Backend, WorkerSnapshot};
use farcaster_storage::{SharedStateStore, StateStore};

use crate::{RuntimeHost, RuntimeMetric, RuntimeTimer};

struct TestHost {
    store: SharedStateStore,
    _directory: Option<tempfile::TempDir>,
}

pub fn host() -> Arc<dyn RuntimeHost> {
    let (directory, path) = if let Some(root) = std::env::var_os("FARCASTER_DATA_DIR") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).expect("create test state directory");
        (None, root.join("state.sqlite3"))
    } else {
        let directory = tempfile::tempdir().expect("create test state directory");
        let path = directory.path().join("state.sqlite3");
        (Some(directory), path)
    };
    Arc::new(TestHost {
        store: SharedStateStore::new(StateStore::open_at(&path).expect("open test state")),
        _directory: directory,
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
        sessions: &[(Backend, PathBuf)],
    ) -> Result<usize, String> {
        farcaster_mcp_server::stop_session_family_workers(project, sessions)
    }

    fn finish_session_family_worker_stop(
        &self,
        project: &Path,
        sessions: &[(Backend, PathBuf)],
    ) -> Result<(), String> {
        farcaster_mcp_server::finish_session_family_worker_stop(project, sessions)
    }

    fn worker_snapshots(&self) -> Result<Vec<WorkerSnapshot>, String> {
        farcaster_mcp_server::worker_snapshots()
    }

    fn contains_invocation(&self, _input: &str, _commands: &[SlashCommand]) -> bool {
        false
    }

    fn count_snapshot(&self) {}

    fn count_stream_event(&self, _coalesced: bool) {}

    fn timer(&self, _metric: RuntimeMetric) -> Box<dyn RuntimeTimer> {
        Box::new(NoopTimer)
    }
}

struct NoopTimer;

impl RuntimeTimer for NoopTimer {}
