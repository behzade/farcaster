use super::*;

fn execution(model: &str) -> WorkerExecution {
    WorkerExecution {
        harness: Backend::Codex,
        provider: "openai".into(),
        model: model.into(),
        effort: Some("medium".into()),
        service_tier: None,
    }
}

#[test]
fn built_ins_have_limits_and_no_default_models() {
    let profiles = WorkerProfiles::default();
    assert_eq!(
        profiles
            .profiles
            .iter()
            .map(|profile| (&*profile.name, profile.limit))
            .collect::<Vec<_>>(),
        [
            ("smartest", 1),
            ("smart", 3),
            ("standard", 10),
            ("light", 20)
        ]
    );
    assert!(
        profiles
            .profiles
            .iter()
            .all(|profile| profile.models.is_empty())
    );
    assert!(profiles.resolve("standard", |_| true).is_err());
}

#[test]
fn one_route_and_positive_limit_are_required() {
    let mut profiles = WorkerProfiles::default();
    profiles.profiles[0].models.push(execution("first"));
    assert_eq!(
        profiles
            .resolve("smartest", |_| true)
            .unwrap()
            .execution
            .model,
        "first"
    );
    profiles.profiles[0].models.push(execution("second"));
    assert!(profiles.validate().is_err());
    profiles.profiles[0].models.pop();
    profiles.profiles[0].limit = 0;
    assert!(profiles.validate().is_err());
    profiles.profiles[0].limit = 1;
    profiles.profiles[0].enabled = false;
    assert!(profiles.resolve("smartest", |_| true).is_err());
}

#[test]
fn saved_ordered_routes_migrate_without_losing_choices() {
    let saved: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/worker_profiles.json"
    ))
    .unwrap();
    let original = saved["profiles"][0]["models"].as_array().unwrap().len();
    let profiles = WorkerProfiles::from_saved(saved).unwrap();
    assert_eq!(
        profiles
            .profiles
            .iter()
            .filter(|profile| profile.name == "oracle" || profile.name.starts_with("oracle_"))
            .count(),
        original
    );
    assert!(
        profiles
            .profiles
            .iter()
            .all(|profile| profile.models.len() <= 1)
    );
    for name in ["smartest", "smart", "standard", "light"] {
        assert!(profiles.profiles.iter().any(|profile| profile.name == name));
    }
    assert_eq!(
        WorkerProfiles::from_saved(serde_json::to_value(&profiles).unwrap()).unwrap(),
        profiles
    );
}

#[test]
fn legacy_tasks_remain_custom_profiles() {
    let route = serde_json::json!({"harness":"pi", "provider":"openai-codex", "model":"test", "effort":null});
    let saved = serde_json::json!({"tasks":[{"name":"audit", "specified":route, "guided":route, "independent":route}]});
    let profiles = WorkerProfiles::from_saved(saved).unwrap();
    assert!(
        profiles
            .profiles
            .iter()
            .any(|profile| profile.name == "audit_specified")
    );
}
