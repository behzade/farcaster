use super::*;
use crate::agents::{
    HarnessAccessMode,
    HarnessAccessMode::{Auto, Full, Sandboxed},
    SessionCommand,
};

#[test]
fn backend_display_names_come_from_descriptors() {
    assert_eq!(backend_display_name("pi"), "Pi");
    assert_eq!(backend_display_name("codex-cli"), "Codex");
    assert_eq!(backend_display_name("cursor-cli"), "Cursor");
    assert_eq!(backend_display_name("opencode2"), "OpenCode");
    assert_eq!(backend_display_name("custom"), "custom");
}

#[test]
fn pi_startup_skips_unsupported_mode_query() {
    assert!(!supports_startup_command("pi", &SessionCommand::ListModes));
}

#[test]
fn backend_access_modes_match_their_native_safety_models() {
    assert_eq!(HarnessAccessMode::default(), Auto);
    assert_eq!(supported_access_modes("pi"), &[Sandboxed, Full]);
    assert_eq!(
        supported_access_modes("codex-cli"),
        &[Sandboxed, Auto, Full]
    );
    assert_eq!(supported_access_modes("cursor-cli"), &[Sandboxed, Full]);
    assert_eq!(supported_access_modes("opencode2"), &[Sandboxed, Full]);
    assert_eq!(normalize_access_mode("opencode2", Auto), Sandboxed);
    assert_eq!(normalize_access_mode("codex-cli", Auto), Auto);
    assert_eq!(normalize_access_mode("cursor-cli", Auto), Sandboxed);
    assert_eq!(normalize_access_mode("pi", Auto), Sandboxed);
}
