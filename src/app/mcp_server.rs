#[cfg(test)]
pub(crate) use farcaster_mcp_server::with_test_worker_pool;
pub(crate) use farcaster_mcp_server::{
    NoticeBoard, NoticeView, finish_session_family_worker_stop, set_worker_app_proxy, start,
    stop_session_family_workers, worker_snapshots,
};

pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    let store = crate::app::persistence::open()?;
    farcaster_mcp_server::set_enabled(enabled, move |enabled| {
        store.save_builtin_mcp_enabled(enabled)
    })
}
