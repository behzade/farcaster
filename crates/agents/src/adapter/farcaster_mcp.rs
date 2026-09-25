pub const URL: &str = "http://127.0.0.1:8765/mcp";
pub const CALLER_HEADER: &str = "farcaster-caller";

pub(super) fn enabled() -> bool {
    crate::builtin_mcp::enabled()
}
