pub use crate::builtin_mcp::url;
pub const CALLER_HEADER: &str = "farcaster-caller";

pub(super) fn enabled() -> bool {
    crate::builtin_mcp::enabled()
}
