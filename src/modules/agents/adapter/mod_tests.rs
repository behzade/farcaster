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
    assert_eq!(backend_display_name("opencode"), "OpenCode");
    assert_eq!(backend_display_name("custom"), "custom");
}

#[test]
fn pi_startup_skips_unsupported_mode_query() {
    assert!(!supports_startup_command("pi", &SessionCommand::ListModes));
}

#[test]
fn access_modes_require_both_backend_and_model_support() {
    for backend in ["cursor-cli", "opencode", "claude", "antigravity-acp"] {
        assert_eq!(
            available_access_modes(backend, None, None),
            [Sandboxed, Full],
            "{backend}"
        );
    }
    assert_eq!(
        available_access_modes("codex-cli", None, None),
        [Sandboxed, Auto, Full]
    );
    assert!(available_access_modes("custom", None, None).is_empty());
    assert_eq!(available_access_modes("pi", None, None), [Auto]);
    assert_eq!(
        available_access_modes("pi", None, Some("pi-nono")),
        [Sandboxed, Full]
    );
    assert!(available_access_modes("pi", None, Some("missing")).is_empty());
    let mut model: crate::protocol::Model = serde_json::from_value(serde_json::json!({
        "id":"model", "name":"Model", "provider":"claude"
    }))
    .expect("test operation should succeed");
    assert_eq!(
        available_access_modes("claude", Some(&model), None),
        [Sandboxed, Full]
    );
    model.access_modes = Some(vec![Sandboxed, Auto, Full]);
    assert_eq!(
        available_access_modes("claude", Some(&model), None),
        [Sandboxed, Auto, Full]
    );
    assert_eq!(
        available_access_modes("pi", Some(&model), Some("pi-nono")),
        [Sandboxed, Full]
    );
    model.access_modes = Some(vec![Sandboxed, Full]);
    assert_eq!(
        available_access_modes("claude", Some(&model), None),
        [Sandboxed, Full]
    );
}

#[test]
fn catalog_launch_resolves_only_to_supported_safe_modes() {
    assert_eq!(configuration_access_mode("opencode", Auto), Ok(Sandboxed));
    assert_eq!(
        configuration_access_mode("opencode", Sandboxed),
        Ok(Sandboxed)
    );
    assert_eq!(configuration_access_mode("opencode", Full), Ok(Full));
    assert_eq!(configuration_access_mode("codex-cli", Auto), Ok(Auto));
    assert_eq!(configuration_access_mode("claude", Auto), Ok(Auto));
    assert_eq!(configuration_access_mode("pi", Auto), Ok(Auto));
    assert!(configuration_access_mode("opencode", Auto).unwrap() != Full);
}

#[test]
fn pi_catalog_reports_the_detected_sandbox_adapter() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let script = project.path().join("fake-pi.sh");
    std::fs::write(
        &script,
        include_str!("../../../../tests/fixtures/fake-pi.sh"),
    )
    .map_err(|error| error.to_string())?;
    let config = crate::agents::AgentLaunchConfig::test_script(&script, vec!["sandbox-on".into()]);

    let catalog = load_configuration_catalog(&config, "pi", project.path())?;

    assert_eq!(catalog.sandbox_adapter.as_deref(), Some("pi-nono"));
    assert_eq!(
        available_access_modes("pi", None, catalog.sandbox_adapter.as_deref()),
        [Sandboxed, Full]
    );
    let controls = std::fs::read_to_string(project.path().join("sandbox-controls"))
        .map_err(|error| error.to_string())?;
    assert!(controls.contains("sandboxed"));
    Ok(())
}
