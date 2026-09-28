use std::{path::Path, thread::Thread, time::Instant};

use farcaster_agent_protocol::extensions::SlashCommand;
use farcaster_agents::WorkerSnapshot;
use farcaster_storage::SharedStateStore;

#[derive(Clone, Copy)]
pub enum RuntimeMetric {
    SelectDocument,
    LoadHistory,
    ProjectHistory,
    RuntimeRoute,
    IdleRetirement,
}

pub trait RuntimeTimer {
    fn set_work(&mut self, _work: usize) {}
}

pub trait ScheduledWake: Send {}

pub trait WorkerActivity: Send {
    fn revision(&self) -> u64;
}

impl WorkerActivity for farcaster_agents::WorkerActivitySubscription {
    fn revision(&self) -> u64 {
        self.revision()
    }
}

pub trait RuntimeHost: Send + Sync {
    fn state_store(&self) -> Result<SharedStateStore, String>;
    fn set_worker_app_proxy(&self, proxy: Option<String>) -> Result<(), String>;
    fn stop_session_family_workers(
        &self,
        project: &Path,
        sessions: &[farcaster_sessions::SessionKey],
    ) -> Result<usize, String>;
    fn finish_session_family_worker_stop(
        &self,
        project: &Path,
        sessions: &[farcaster_sessions::SessionKey],
    ) -> Result<(), String>;
    fn worker_snapshots(&self) -> Result<Vec<WorkerSnapshot>, String>;
    fn schedule_wake(&self, deadline: Instant, wake: Thread) -> Box<dyn ScheduledWake>;
    fn subscribe_worker_activity(&self, project: &Path, wake: Thread) -> Box<dyn WorkerActivity> {
        Box::new(farcaster_agents::subscribe_worker_activity(project, wake))
    }
    fn worker_inboxes_idle(&self, project: &Path) -> bool {
        farcaster_agents::CallerRegistry::shared().child_inboxes_idle(project)
    }
    fn contains_invocation(&self, input: &str, commands: &[SlashCommand]) -> bool;
    fn count_snapshot(&self);
    fn count_stream_event(&self, coalesced: bool);
    fn timer(&self, metric: RuntimeMetric) -> Box<dyn RuntimeTimer>;
}
