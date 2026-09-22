pub(crate) use farcaster_mcp_server::{
    NoticeBoard, NoticeView, finish_session_family_worker_stop, set_worker_app_proxy,
    stop_session_family_workers, worker_snapshots,
};

pub(crate) fn start(
    store: crate::app::persistence::SharedStateStore,
    workers: crate::agents::WorkerPool,
    updates: async_channel::Sender<()>,
    notices: NoticeBoard,
) -> Result<farcaster_mcp_server::McpServer, String> {
    farcaster_mcp_server::start(store.arc(), workers, updates, notices)
}

pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    farcaster_mcp_server::set_enabled(enabled, move |enabled| {
        crate::app::persistence::open()?.save_builtin_mcp_enabled(enabled)
    })
}
