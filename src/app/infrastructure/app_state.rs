use crate::app::*;

pub(in crate::app) struct AppLifecycle {
    pub(in crate::app) performance_monitor: Option<infrastructure::performance::PerformanceMonitor>,
    pub(in crate::app) pending_session_switch:
        Option<(PathBuf, infrastructure::performance::Timing)>,
    pub(in crate::app) pending_quit: Option<infrastructure::quit::PendingQuit>,
    pub(in crate::app) _performance_task: Option<Task<()>>,
    pub(in crate::app) _window_placement_subscription: Subscription,
    pub(in crate::app) _event_task: Task<()>,
    pub(in crate::app) _workgraph_update_task: Task<()>,
    pub(in crate::app) _worker_update_task: Task<()>,
    pub(in crate::app) _worker_notice_task: Task<()>,
}
