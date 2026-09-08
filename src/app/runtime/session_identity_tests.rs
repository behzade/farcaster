use super::*;

fn model(id: &str, reasoning: bool, efforts: Option<&[&str]>) -> Model {
    Model {
        id: id.into(),
        name: id.into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning,
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
            harness: "cursor-cli".into(),
            model: None,
            effort: Some("off".into()),
        },
    ]);
    assert_eq!(defaults.effort("cursor-cli"), None);
    assert!(!defaults.set_effort("cursor-cli", "high".into()));

    let mut ready = RuntimeSnapshot {
        harness: "cursor-cli".into(),
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
    assert_eq!(defaults.effort("cursor-cli"), None);
    assert!(defaults.cached().is_empty());

    let mut draft = RuntimeSnapshot {
        harness: "cursor-cli".into(),
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
