use super::*;

fn historical_profiles() -> WorkerProfiles {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/worker_profiles.json"
    ))
    .expect("historical worker profile fixture must be valid")
}

#[test]
fn empty_is_deliberate_and_invalid_definitions_fail_closed() {
    let empty = WorkerProfiles { profiles: vec![] };
    assert!(empty.validate().is_ok());
    assert!(empty.resolve("fast", |_| true).is_err());
    assert_eq!(WorkerProfiles::default(), empty);
    let mut profiles = historical_profiles();
    profiles.profiles[0].models[0].model.clear();
    assert!(profiles.validate().is_err());
    let mut profiles = historical_profiles();
    profiles.profiles[0].name = "audit".into();
    assert!(profiles.resolve("oracle", |_| true).is_err());
    assert_eq!(
        profiles
            .resolve("audit", |_| true)
            .expect("test operation should succeed")
            .profile,
        "audit"
    );
    profiles.profiles[1].name = "AUDIT".into();
    assert!(profiles.validate().is_err());
    let mut profiles = historical_profiles();
    profiles.profiles[0].description.clear();
    assert!(profiles.validate().is_err());
    assert!(WorkerProfiles::from_saved(serde_json::json!({"unexpected": []})).is_err());
    let mut profiles = historical_profiles();
    profiles.profiles[0].models.clear();
    assert!(profiles.validate().is_err());
}

#[test]
fn model_order_controls_selection() {
    let mut profiles = historical_profiles();
    assert!(profiles.resolve("fast", |_| false).is_err());
    let expected = profiles.profiles[1].models[1].clone();
    profiles.profiles[1].models.swap(0, 1);
    assert_eq!(
        profiles
            .resolve("fast", |_| true)
            .expect("test operation should succeed")
            .execution,
        expected
    );
}

#[test]
fn saved_profiles_migrate_deprecated_cursor_model_ids() {
    let mut saved =
        serde_json::to_value(historical_profiles()).expect("test operation should succeed");
    saved["profiles"][0]["models"][3]["model"] = "grok-4.6[effort=high,fast=true]".into();
    saved["profiles"][2]["models"][2]["model"] = "composer-2.5[fast=true]".into();
    saved["profiles"][2]["models"][2]["provider"] = "custom".into();
    saved["profiles"][3]["models"][4]["model"] = "composer-2.5[fast=true]".into();

    let profiles = WorkerProfiles::from_saved(saved).expect("test operation should succeed");
    assert_eq!(profiles.profiles[0].models[3].model, "grok-4.6");
    assert_eq!(
        profiles.profiles[2].models[2].model,
        "composer-2.5[fast=true]"
    );
    assert_eq!(profiles.profiles[3].models[4].model, "composer-2.5");
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
fn legacy_routes_become_profiles_but_empty_stays_empty() {
    let saved = serde_json::json!({"tasks": [legacy_task("read"), legacy_task("implement"), legacy_task("review")]});
    let migrated = WorkerProfiles::from_saved(saved).expect("test operation should succeed");
    assert_eq!(migrated.profiles.len(), 3);
    assert_eq!(migrated.profiles[0].name, "read_specified");
    assert_eq!(migrated.profiles[1].name, "read_guided");
    assert_eq!(migrated.profiles[2].name, "read_independent");
    assert_eq!(migrated.profiles[0].models[0].model, "gpt-5.6-luna");
    assert_eq!(migrated.profiles[1].models[0].model, "gpt-5.6-sol");
    assert_eq!(migrated.profiles[2].models[0].model, "gpt-6-astra");
    assert!(
        WorkerProfiles::from_saved(serde_json::json!({"tasks": []}))
            .expect("test operation should succeed")
            .profiles
            .is_empty()
    );
}

#[test]
fn migration_preserves_distinct_custom_routes_without_duplicates() {
    let mut task = legacy_task("implement");
    let defaults = historical_profiles();
    task["independent"] = serde_json::to_value(
        &defaults
            .resolve("fast", |_| true)
            .expect("test operation should succeed")
            .execution,
    )
    .expect("test operation should succeed");
    task["guided"] = serde_json::to_value(
        &defaults
            .resolve("cheap", |_| true)
            .expect("test operation should succeed")
            .execution,
    )
    .expect("test operation should succeed");
    task["specified"] = task["guided"].clone();
    task["specified"]["effort"] = "high".into();
    let result = WorkerProfiles::from_saved(serde_json::json!({"tasks": [task]}))
        .expect("test operation should succeed");
    assert_eq!(result.profiles.len(), 3);
    assert_eq!(
        result
            .resolve("implement_specified", |_| true)
            .expect("test operation should succeed")
            .execution
            .effort
            .as_deref(),
        Some("high")
    );
    assert_eq!(
        WorkerProfiles::from_saved(
            serde_json::to_value(&result).expect("test operation should succeed")
        )
        .expect("test operation should succeed"),
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
    let result = WorkerProfiles::from_saved(serde_json::json!({"tasks": [first, second]}))
        .expect("test operation should succeed");
    assert_eq!(result.profiles.len(), 4);
    assert_ne!(result.profiles[0].name, result.profiles[3].name);
    let mut invalid = legacy_task("read");
    invalid["guided"]["provider"] = "".into();
    assert!(WorkerProfiles::from_saved(serde_json::json!({"tasks": [invalid]})).is_err());
}

#[test]
fn new_profile_is_an_empty_draft_and_inherit_is_reserved() {
    assert_eq!(
        WorkerProfile::new("audit".into()),
        WorkerProfile {
            name: "audit".into(),
            description: String::new(),
            models: Vec::new(),
        }
    );
    for name in ["inherit", "INHERIT", "InHeRiT"] {
        let mut profile = historical_profiles().profiles.remove(0);
        profile.name = name.into();
        assert!(
            WorkerProfiles {
                profiles: vec![profile]
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn saved_profiles_survive_empty_defaults_and_reserved_name_migration() {
    let profiles = historical_profiles();
    let saved = serde_json::to_value(&profiles).expect("test operation should succeed");
    assert_eq!(
        WorkerProfiles::from_saved(saved).expect("test operation should succeed"),
        profiles
    );

    let mut saved =
        serde_json::to_value(historical_profiles()).expect("test operation should succeed");
    saved["profiles"][0]["name"] = "INHERIT".into();
    saved["profiles"][1]["name"] = "inherit_custom".into();
    let migrated = WorkerProfiles::from_saved(saved).expect("test operation should succeed");
    assert_eq!(migrated.profiles[0].name, "inherit_custom_1");
    assert_eq!(migrated.profiles[1].name, "inherit_custom");
}
