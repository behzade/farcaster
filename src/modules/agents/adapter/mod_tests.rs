use super::*;
use crate::agents::{
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
fn access_modes_require_both_backend_and_model_support() {
    for backend in ["pi", "cursor-cli", "opencode2", "claude", "antigravity-acp"] {
        assert_eq!(
            available_access_modes(backend, None),
            [Sandboxed, Full],
            "{backend}"
        );
    }
    assert_eq!(
        available_access_modes("codex-cli", None),
        [Sandboxed, Auto, Full]
    );
    assert_eq!(available_access_modes("custom", None), [Full]);
    let mut model: crate::protocol::Model = serde_json::from_value(serde_json::json!({
        "id":"model", "name":"Model", "provider":"claude"
    }))
    .unwrap();
    assert_eq!(
        available_access_modes("claude", Some(&model)),
        [Sandboxed, Full]
    );
    model.access_modes = Some(vec![Sandboxed, Auto, Full]);
    assert_eq!(
        available_access_modes("claude", Some(&model)),
        [Sandboxed, Auto, Full]
    );
    assert_eq!(
        available_access_modes("pi", Some(&model)),
        [Sandboxed, Full]
    );
    model.access_modes = Some(vec![Sandboxed, Full]);
    assert_eq!(
        available_access_modes("claude", Some(&model)),
        [Sandboxed, Full]
    );
}
