use super::*;

fn model(id: &str, reasoning: bool, efforts: Option<&[&str]>) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning,
        resolved_model: None,
        access_modes: None,
        efforts: efforts.map(|efforts| efforts.iter().map(|effort| (*effort).into()).collect()),
    }
}

#[test]
fn available_thinking_levels_follow_the_selected_model() {
    let selected = model("selected", true, Some(&["low", "medium"]));
    let snapshot = RuntimeSnapshot {
        prefill_model: Some(selected.clone()),
        models: vec![selected, model("other", true, Some(&["high", "xhigh"]))],
        thinking_levels: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
        ..RuntimeSnapshot::default()
    };

    assert_eq!(snapshot.available_thinking_levels(), ["low", "medium"]);
}

#[test]
fn available_thinking_levels_use_the_first_model_for_a_new_draft() {
    let snapshot = RuntimeSnapshot {
        models: vec![model("default", true, Some(&["minimal", "low"]))],
        thinking_levels: vec!["minimal".into(), "low".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };

    assert_eq!(snapshot.available_thinking_levels(), ["minimal", "low"]);
}

#[test]
fn cached_catalog_replaces_a_resident_loading_or_stale_snapshot() {
    let mut store = HarnessConfigurationStore::default();
    let mut snapshot = RuntimeSnapshot {
        harness: "cursor-cli".into(),
        project: PathBuf::from("/project"),
        ..RuntimeSnapshot::default()
    };
    let loaded = model("loaded", false, None);
    store.set_catalog(
        snapshot.harness.clone(),
        snapshot.project.clone(),
        crate::agents::ConfigurationCatalog {
            models: vec![loaded.clone()],
            efforts: vec![],
            sandbox_adapter: None,
        },
    );
    store.refresh_snapshot_catalog(&mut snapshot);
    assert_eq!(snapshot.models, vec![loaded.clone()]);
    assert_eq!(snapshot.configuration_status, ConfigurationStatus::Loaded);
    snapshot.models = vec![model("stale", false, None)];
    store.refresh_snapshot_catalog(&mut snapshot);
    assert_eq!(snapshot.models, vec![loaded]);
    snapshot.harness = "pi".into();
    snapshot.models.clear();
    store.refresh_snapshot_catalog(&mut snapshot);
    assert!(snapshot.models.is_empty());
}

#[test]
fn pi_catalog_capability_reaches_a_model_less_draft() {
    use crate::agents::HarnessAccessMode::{Full, Sandboxed};

    let project = PathBuf::from("/project");
    let mut store = HarnessConfigurationStore::default();
    store.set_catalog(
        "pi".into(),
        project.clone(),
        crate::agents::ConfigurationCatalog {
            models: vec![],
            efforts: vec!["off".into()],
            sandbox_adapter: Some("pi-nono".into()),
        },
    );
    assert!(store.catalog_command("pi", &project).is_some());

    let mut snapshot = RuntimeSnapshot {
        harness: "pi".into(),
        project,
        ..RuntimeSnapshot::default()
    };
    store.refresh_snapshot_catalog(&mut snapshot);

    assert_eq!(snapshot.sandbox_adapter.as_deref(), Some("pi-nono"));
    assert_eq!(snapshot.available_access_modes(), [Sandboxed, Full]);
}

#[test]
fn available_thinking_levels_keep_legacy_global_catalogs() {
    let snapshot = RuntimeSnapshot {
        prefill_model: Some(model("legacy", true, None)),
        thinking_levels: vec!["off".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };

    assert_eq!(snapshot.available_thinking_levels(), ["off", "high"]);
}

#[test]
fn known_empty_model_efforts_do_not_fall_back_to_other_models() {
    let snapshot = RuntimeSnapshot {
        prefill_model: Some(model("fixed", true, Some(&[]))),
        thinking_levels: vec!["low".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };

    assert!(snapshot.available_thinking_levels().is_empty());
}

#[test]
fn non_reasoning_models_have_no_effort_choices() {
    let snapshot = RuntimeSnapshot {
        prefill_model: Some(model("plain", false, None)),
        thinking_levels: vec!["off".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };

    assert!(snapshot.available_thinking_levels().is_empty());
}

#[test]
fn session_default_is_not_replaced_by_stale_draft_effort() {
    let snapshot = RuntimeSnapshot {
        harness: "opencode".into(),
        prefill_thinking_level: Some("high".into()),
        session: Some(
            serde_json::from_value(serde_json::json!({
                "isStreaming": false, "isCompacting": false, "sessionId": "ses_default",
                "autoCompactionEnabled": false, "messageCount": 0, "pendingMessageCount": 0
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    assert_eq!(snapshot.session_identity().effort, None);
}

#[test]
fn cleared_default_stays_unset_after_configuration_restore() {
    let mut store = HarnessConfigurationStore::default();
    store.set_model("opencode", model("astra", true, Some(&["low", "high"])));
    store.set_effort("opencode", "high".into());
    assert!(store.reset_effort("opencode"));
    assert!(!store.reset_effort("opencode"));
    let mut restored = HarnessConfigurationStore::default();
    restored.restore(store.cached());
    let mut draft = RuntimeSnapshot {
        harness: "opencode".into(),
        ..Default::default()
    };
    restored.reconcile_snapshot(&mut draft, true);
    assert_eq!(draft.session_identity().effort, None);
    assert_eq!(draft.session_identity().model.unwrap().id, "astra");
}

#[test]
fn selecting_model_replaces_an_unsupported_cached_effort_with_the_nearest_level() {
    let mut defaults = HarnessConfigurationStore::default();
    assert!(defaults.set_effort("pi", "high".into()));
    assert!(defaults.set_model(
        "pi",
        model("limited", true, Some(&["off", "low", "medium"])),
    ));

    let mut draft = RuntimeSnapshot {
        harness: "pi".into(),
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut draft, true);

    assert_eq!(draft.prefill_thinking_level.as_deref(), Some("medium"));
}

#[test]
fn unsupported_reasoning_is_not_restored_or_cached() {
    let mut defaults = HarnessConfigurationStore::default();
    defaults.restore(vec![
        crate::app::infrastructure::persistence::CachedSessionControlDefaults {
            harness: "unknown".into(),
            model: None,
            effort: Some("off".into()),
        },
    ]);
    assert_eq!(defaults.effort("unknown"), None);
    assert!(!defaults.set_effort("unknown", "high".into()));

    let mut ready = RuntimeSnapshot {
        harness: "unknown".into(),
        session: Some(
            serde_json::from_value(serde_json::json!({
                "thinkingLevel": "off",
                "isStreaming": false,
                "isCompacting": false,
                "sessionId": "cursor-session",
                "autoCompactionEnabled": false,
                "messageCount": 0,
                "pendingMessageCount": 0
            }))
            .expect("session state"),
        ),
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut ready, true);
    assert_eq!(defaults.effort("unknown"), None);
    assert!(defaults.cached().is_empty());

    let mut draft = RuntimeSnapshot {
        harness: "unknown".into(),
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut draft, true);
    assert_eq!(draft.prefill_thinking_level, None);
}

#[test]
fn cached_defaults_restore_across_projects_per_harness() {
    let selected = model("selected", true, Some(&["low", "high"]));
    let mut defaults = HarnessConfigurationStore::default();
    assert!(defaults.set_model("codex-cli", selected.clone()));
    assert!(defaults.set_effort("codex-cli", "high".into()));

    let mut restarted = HarnessConfigurationStore::default();
    restarted.restore(defaults.cached());
    let mut draft = RuntimeSnapshot {
        harness: "codex-cli".into(),
        project: PathBuf::from("/another-project"),
        ..RuntimeSnapshot::default()
    };
    restarted.reconcile_snapshot(&mut draft, true);

    assert_eq!(draft.prefill_model, Some(selected));
    assert_eq!(draft.prefill_thinking_level.as_deref(), Some("high"));

    let mut other_harness = RuntimeSnapshot {
        harness: "pi".into(),
        project: PathBuf::from("/another-project"),
        ..RuntimeSnapshot::default()
    };
    restarted.reconcile_snapshot(&mut other_harness, true);
    assert_eq!(other_harness.prefill_model, None);
    assert_eq!(other_harness.prefill_thinking_level, None);
}

#[test]
fn available_access_modes_use_fresh_catalog_support_for_selected_model() {
    use crate::agents::HarnessAccessMode::{Auto, Full, Sandboxed};
    for catalog_id in ["selected", "alias"] {
        let selected = model("selected", false, None);
        let mut supported = model(catalog_id, false, None);
        supported.resolved_model = Some(selected.id.clone());
        supported.access_modes = Some(vec![Sandboxed, Auto, Full]);
        let mut snapshot = RuntimeSnapshot {
            harness: "claude".into(),
            prefill_model: Some(selected),
            models: vec![supported],
            ..RuntimeSnapshot::default()
        };
        assert_eq!(snapshot.available_access_modes(), [Sandboxed, Auto, Full]);
        snapshot.models[0].access_modes = Some(vec![Sandboxed, Full]);
        assert_eq!(snapshot.available_access_modes(), [Sandboxed, Full]);
        snapshot.models.clear();
        assert_eq!(snapshot.available_access_modes(), [Sandboxed, Full]);
    }
}
