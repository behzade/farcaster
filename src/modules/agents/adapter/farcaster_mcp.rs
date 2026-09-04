pub(crate) const URL: &str = "http://127.0.0.1:8765/mcp";
pub(crate) const CALLER_HEADER: &str = "farcaster-caller";

pub(super) fn enabled() -> bool {
    crate::builtin_mcp::enabled()
}

#[cfg(test)]
pub(super) fn set_enabled(enabled: bool) {
    crate::builtin_mcp::set_enabled(enabled);
}
pub(crate) const INSTRUCTIONS: &str = "Farcaster provides parent-child workers, a coordination notice board, and durable work graphs.";
