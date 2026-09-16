use super::*;
use crate::agents::Backend;
use crate::agents::{
    HarnessAccessMode::{Auto, Full, Sandboxed},
    SessionCommand,
};

#[test]
fn backend_display_names_come_from_descriptors() {
    assert_eq!(backend_display_name(Some(Backend::Pi)), "Pi");
    assert_eq!(backend_display_name(Some(Backend::Codex)), "Codex");
    assert_eq!(backend_display_name(Some(Backend::Cursor)), "Cursor");
    assert_eq!(backend_display_name(Some(Backend::OpenCode)), "OpenCode");
    assert_eq!(backend_display_name(None), "Choose a backend");
}

#[test]
fn pi_startup_skips_unsupported_mode_query() {
    assert!(!supports_startup_command(
        Some(Backend::Pi),
        &SessionCommand::ListModes
    ));
}

#[test]
fn access_modes_require_both_backend_and_model_support() {
    for backend in [
        Backend::Cursor,
        Backend::OpenCode,
        Backend::Claude,
        Backend::Antigravity,
    ] {
        assert_eq!(
            available_access_modes(backend, None, None),
            [Sandboxed, Full],
            "{backend}"
        );
    }
    assert_eq!(
        available_access_modes(Some(Backend::Codex), None, None),
        [Sandboxed, Auto, Full]
    );
    assert!(available_access_modes(None, None, None).is_empty());
    assert_eq!(
        available_access_modes(Some(Backend::Pi), None, None),
        [Auto]
    );
    assert_eq!(
        available_access_modes(Some(Backend::Pi), None, Some("pi-nono")),
        [Sandboxed, Full]
    );
    assert!(available_access_modes(Some(Backend::Pi), None, Some("missing")).is_empty());
    let mut model: crate::protocol::Model = serde_json::from_value(serde_json::json!({
        "id":"model", "name":"Model", "provider":"claude"
    }))
    .expect("test operation should succeed");
    assert_eq!(
        available_access_modes(Some(Backend::Claude), Some(&model), None),
        [Sandboxed, Full]
    );
    model.access_modes = Some(vec![Sandboxed, Auto, Full]);
    assert_eq!(
        available_access_modes(Some(Backend::Claude), Some(&model), None),
        [Sandboxed, Auto, Full]
    );
    assert_eq!(
        available_access_modes(Some(Backend::Pi), Some(&model), Some("pi-nono")),
        [Sandboxed, Full]
    );
    model.access_modes = Some(vec![Sandboxed, Full]);
    assert_eq!(
        available_access_modes(Some(Backend::Claude), Some(&model), None),
        [Sandboxed, Full]
    );
}

#[test]
fn catalog_launch_resolves_only_to_supported_safe_modes() {
    assert_eq!(
        configuration_access_mode(Backend::OpenCode, Auto),
        Ok(Sandboxed)
    );
    assert_eq!(
        configuration_access_mode(Backend::OpenCode, Sandboxed),
        Ok(Sandboxed)
    );
    assert_eq!(configuration_access_mode(Backend::OpenCode, Full), Ok(Full));
    assert_eq!(configuration_access_mode(Backend::Codex, Auto), Ok(Auto));
    assert_eq!(configuration_access_mode(Backend::Claude, Auto), Ok(Auto));
    assert_eq!(configuration_access_mode(Backend::Pi, Auto), Ok(Auto));
    assert!(
        configuration_access_mode(Backend::OpenCode, Auto).expect("OpenCode auto mode") != Full
    );
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

    let catalog = load_configuration_catalog(&config, Backend::Pi, project.path())?;

    assert_eq!(catalog.sandbox_adapter.as_deref(), Some("pi-nono"));
    assert_eq!(
        available_access_modes(Some(Backend::Pi), None, catalog.sandbox_adapter.as_deref()),
        [Sandboxed, Full]
    );
    let controls = std::fs::read_to_string(project.path().join("sandbox-controls"))
        .map_err(|error| error.to_string())?;
    assert!(controls.contains("sandboxed"));
    Ok(())
}

#[test]
fn backend_descriptors_and_worker_factories_cover_every_variant() {
    let descriptors = known_backend_descriptors();
    let (factories, default) = worker_factories(crate::agents::AgentLaunchConfig::default());
    assert_eq!(default, Backend::Pi);
    assert_eq!(factories.len(), Backend::ALL.len());
    for backend in Backend::ALL {
        assert_eq!(
            descriptors
                .iter()
                .filter(|entry| entry.id == backend)
                .count(),
            1
        );
        assert!(factories.contains_key(&backend));
    }
}
