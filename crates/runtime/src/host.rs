use std::path::{Path, PathBuf};

use farcaster_agent_protocol::extensions::SlashCommand;
use farcaster_agents::{Backend, WorkerSnapshot};
use farcaster_storage::SharedStateStore;

#[derive(Clone, Copy)]
pub enum RuntimeMetric {
    SelectDocument,
    LoadHistory,
    ProjectHistory,
    RuntimeRoute,
}

pub trait RuntimeTimer {
    fn set_work(&mut self, _work: usize) {}
}

pub trait RuntimeHost: Send + Sync {
    fn state_store(&self) -> Result<SharedStateStore, String>;
    fn set_worker_app_proxy(&self, proxy: Option<String>) -> Result<(), String>;
    fn stop_session_family_workers(
        &self,
        project: &Path,
        sessions: &[(Backend, PathBuf)],
    ) -> Result<usize, String>;
    fn finish_session_family_worker_stop(
        &self,
        project: &Path,
        sessions: &[(Backend, PathBuf)],
    ) -> Result<(), String>;
    fn worker_snapshots(&self) -> Result<Vec<WorkerSnapshot>, String>;
    fn contains_invocation(&self, input: &str, commands: &[SlashCommand]) -> bool;
    fn count_snapshot(&self);
    fn count_stream_event(&self, coalesced: bool);
    fn timer(&self, metric: RuntimeMetric) -> Box<dyn RuntimeTimer>;
}
