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
pub(crate) const INSTRUCTIONS: &str = "Farcaster provides named parent-child workers, a non-intrusive notice board for top-level coordination, and durable work graphs. A top-level worker uses worker_send with a concise child name such as `diff-review`; first use creates that child and later uses message it. A child worker's worker_send schema omits `to` because messages always go to its parent. Top-level workers may use worker_notices only when shared-worktree changes overlap or may conflict with their work, or when change ownership is unclear; do not consult or post for unrelated changes. Notice-board access never pushes messages into another agent's workflow. Use the harness's native subagents when the harness provides them.";
