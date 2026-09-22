use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use farcaster_runtime::{RuntimeHost, RuntimeMetric, RuntimeTimer};

use crate::{
    agents::{Backend, WorkerSnapshot},
    protocol::SlashCommand,
};

pub(crate) struct AppRuntimeHost;

impl RuntimeHost for AppRuntimeHost {
    fn state_store(&self) -> Result<farcaster_storage::SharedStateStore, String> {
        super::persistence::shared()
    }

    fn set_worker_app_proxy(&self, proxy: Option<String>) -> Result<(), String> {
        super::mcp_server::set_worker_app_proxy(proxy)
    }

    fn stop_session_family_workers(
        &self,
        project: &Path,
        sessions: &[(Backend, PathBuf)],
    ) -> Result<usize, String> {
        super::mcp_server::stop_session_family_workers(project, sessions)
    }

    fn finish_session_family_worker_stop(
        &self,
        project: &Path,
        sessions: &[(Backend, PathBuf)],
    ) -> Result<(), String> {
        super::mcp_server::finish_session_family_worker_stop(project, sessions)
    }

    fn worker_snapshots(&self) -> Result<Vec<WorkerSnapshot>, String> {
        super::mcp_server::worker_snapshots()
    }

    fn contains_invocation(&self, input: &str, commands: &[SlashCommand]) -> bool {
        super::composer::user_invocations::contains_invocation(input, commands)
    }

    fn count_snapshot(&self) {
        super::infrastructure::performance::count_snapshot();
    }

    fn count_stream_event(&self, coalesced: bool) {
        super::infrastructure::performance::count_stream_event(coalesced);
    }

    fn timer(&self, metric: RuntimeMetric) -> Box<dyn RuntimeTimer> {
        use super::infrastructure::performance::{OperationKind, OperationTiming, Timing};
        let name = match metric {
            RuntimeMetric::SelectDocument => "switch.select_document",
            RuntimeMetric::LoadHistory => "switch.load_history",
            RuntimeMetric::RuntimeRoute => "switch.runtime_route",
        };
        let operation = matches!(metric, RuntimeMetric::LoadHistory)
            .then(|| OperationTiming::new(OperationKind::HistoryLoad, 0));
        Box::new(AppRuntimeTimer {
            _timing: Timing::new(name),
            operation,
        })
    }
}

struct AppRuntimeTimer {
    _timing: super::infrastructure::performance::Timing,
    operation: Option<super::infrastructure::performance::OperationTiming>,
}

impl RuntimeTimer for AppRuntimeTimer {
    fn set_work(&mut self, work: usize) {
        if let Some(operation) = &mut self.operation {
            operation.set_work(work);
        }
    }
}

pub(crate) fn host() -> Arc<dyn RuntimeHost> {
    Arc::new(AppRuntimeHost)
}
