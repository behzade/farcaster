use super::*;

#[test]
fn empty_is_deliberate_and_invalid_definitions_fail_closed() {
    let empty = WorkerProfiles { profiles: vec![] };
    assert!(empty.validate().is_ok());
    assert!(empty.resolve("fast", |_| true).is_err());
    let mut profiles = WorkerProfiles::default();
    profiles.profiles[0].models[0].model.clear();
    assert!(profiles.validate().is_err());
    let mut profiles = WorkerProfiles::default();
    profiles.profiles[0].name = "audit".into();
    assert!(profiles.resolve("oracle", |_| true).is_err());
    assert_eq!(
        profiles.resolve("audit", |_| true).unwrap().profile,
        "audit"
    );
    profiles.profiles[1].name = "AUDIT".into();
    assert!(profiles.validate().is_err());
    let mut profiles = WorkerProfiles::default();
    profiles.profiles[0].description.clear();
    assert!(profiles.validate().is_err());
    assert!(WorkerProfiles::from_saved(serde_json::json!({"unexpected": []})).is_err());
    let mut profiles = WorkerProfiles::default();
    profiles.profiles[0].models.clear();
    assert!(profiles.validate().is_err());
}

#[test]
fn every_default_profile_works_with_any_single_harness() {
    let profiles = WorkerProfiles::default();
    for harness in ["pi", "codex-cli", "cursor-cli", "opencode2"] {
        for profile in &profiles.profiles {
            let assignment = profiles
                .resolve(&profile.name, |model| model.harness == harness)
                .unwrap();
            assert_eq!(assignment.execution.harness, harness);
        }
    }
    assert!(
        profiles
            .resolve("fast", |_| false)
            .unwrap_err()
            .contains("no available model")
    );
}

#[test]
fn model_order_controls_selection() {
    let mut profiles = WorkerProfiles::default();
    let expected = profiles.profiles[1].models[1].clone();
    profiles.profiles[1].models.swap(0, 1);
    assert_eq!(
        profiles.resolve("fast", |_| true).unwrap().execution,
        expected
    );
}

fn legacy_task(name: &str) -> serde_json::Value {
    let execution = |model, effort| {
        serde_json::json!({
            "harness": "pi", "provider": "openai-codex", "model": model, "effort": effort
        })
    };
    serde_json::json!({
        "name": name,
        "specified": execution("gpt-5.6-luna", "high"),
        "guided": execution("gpt-5.6-sol", "medium"),
        "independent": execution("gpt-6-astra", "medium"),
    })
}

#[test]
fn legacy_defaults_become_profiles_but_empty_stays_empty() {
    let saved = serde_json::json!({"tasks": [legacy_task("read"), legacy_task("implement"), legacy_task("review")]});
    assert_eq!(
        WorkerProfiles::from_saved(saved).unwrap(),
        WorkerProfiles::default()
    );
    assert!(
        WorkerProfiles::from_saved(serde_json::json!({"tasks": []}))
            .unwrap()
            .profiles
            .is_empty()
    );
}

#[test]
fn migration_preserves_distinct_custom_routes_without_duplicate_presets() {
    let mut task = legacy_task("implement");
    let defaults = WorkerProfiles::default();
    task["independent"] =
        serde_json::to_value(&defaults.resolve("fast", |_| true).unwrap().execution).unwrap();
    task["guided"] =
        serde_json::to_value(&defaults.resolve("cheap", |_| true).unwrap().execution).unwrap();
    task["specified"] = task["guided"].clone();
    task["specified"]["effort"] = "high".into();
    let result = WorkerProfiles::from_saved(serde_json::json!({"tasks": [task]})).unwrap();
    assert_eq!(result.profiles.len(), 5);
    assert_eq!(
        result
            .resolve("implement_specified", |_| true)
            .unwrap()
            .execution
            .effort
            .as_deref(),
        Some("high")
    );
    assert_eq!(
        WorkerProfiles::from_saved(serde_json::to_value(&result).unwrap()).unwrap(),
        result
    );
}

#[test]
fn migration_handles_long_names_and_rejects_invalid_legacy_routes() {
    let name = "a".repeat(48);
    let mut first = legacy_task(&name);
    first["specified"]["model"] = "first".into();
    let mut second = legacy_task(&format!("{}b", "a".repeat(47)));
    second["specified"]["model"] = "second".into();
    let result = WorkerProfiles::from_saved(serde_json::json!({"tasks": [first, second]})).unwrap();
    assert_eq!(result.profiles.len(), 6);
    assert_ne!(result.profiles[4].name, result.profiles[5].name);
    let mut invalid = legacy_task("read");
    invalid["guided"]["provider"] = "".into();
    assert!(WorkerProfiles::from_saved(serde_json::json!({"tasks": [invalid]})).is_err());
}
