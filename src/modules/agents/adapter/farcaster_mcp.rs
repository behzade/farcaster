pub(crate) const URL: &str = "http://127.0.0.1:8765/mcp";
pub(crate) const CALLER_HEADER: &str = "farcaster-caller";

#[cfg(test)]
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(super) fn enabled() -> bool {
    #[cfg(test)]
    return ENABLED.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    true
}

#[cfg(test)]
pub(super) fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}
pub(crate) const INSTRUCTIONS: &str = "Farcaster provides project-scoped communication between top-level peer workers and durable work graphs. worker_list returns other top-level peers in this project; child workers see only their parent. Use worker_send with `to: new` only for substantial independent work; it creates an independent top-level agent in this project using this harness and model. Use `to: child` for delegated subtasks such as review. Child workers can only message their parent (`to: parent`). Use the harness's native subagents when the harness provides them.";
