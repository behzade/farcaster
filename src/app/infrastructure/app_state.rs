use crate::app::*;

pub(in crate::app) struct AppLifecycle {
    pub(in crate::app) switch_paint:
        Option<std::sync::Arc<crate::app::infrastructure::performance::SwitchPaintTrace>>,
    pub(in crate::app) performance_monitor: Option<infrastructure::performance::PerformanceMonitor>,
    pub(in crate::app) pending_session_switch:
        Option<(PathBuf, infrastructure::performance::Timing)>,
    pub(in crate::app) opening_local: Option<Task<()>>,
    /// Advances at user selection, before local reads or catalog retries can route it.
    pub(in crate::app) navigation_intent: u64,
    pub(in crate::app) switch_sequence: u64,
    pub(in crate::app) awaiting_switch_route: Option<u64>,
    pub(in crate::app) awaiting_draft_snapshot: Option<String>,
    pub(in crate::app) switch_recovery: Option<session::lifecycle::SwitchRecovery>,
    pub(in crate::app) requested_session_key: Option<crate::sessions::SessionKey>,
    pub(in crate::app) saving_before_quit: bool,
    pub(in crate::app) pending_quit: Option<infrastructure::quit::PendingQuit>,
    pub(in crate::app) _performance_task: Option<Task<()>>,
    pub(in crate::app) _window_placement_subscription: Subscription,
    pub(in crate::app) _event_task: Task<()>,
    pub(in crate::app) _workgraph_update_task: Task<()>,
    pub(in crate::app) _worker_update_task: Task<()>,
    pub(in crate::app) _worker_notice_task: Task<()>,
}
