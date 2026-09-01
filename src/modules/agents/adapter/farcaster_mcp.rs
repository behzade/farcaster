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
pub(crate) const INSTRUCTIONS: &str = "Farcaster provides communication between top-level peer workers and durable work graphs. Use worker_send with `to: new` only for substantial independent work; it creates an independent top-level agent using this harness and model. Use the harness's native subagents for delegated subtasks.";
