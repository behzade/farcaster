use super::*;

#[test]
fn environment_connections_follow_the_project_override_and_native_stores_stay_shared() {
    let config = AgentLaunchConfig::default();
    let first = Path::new("/first-project");
    let second = Path::new("/second-project");
    let mut command = std::process::Command::new("unused");
    command.env_clear().env("HOME", "/fixture-home");
    assert_eq!(
        source_from_command(&config, first, &command).key(),
        source_from_command(&config, second, &command).key()
    );
    for name in ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"] {
        command.env(name, "fixture-credential-a");
        let a = source_from_command(&config, first, &command);
        command.env(name, "fixture-credential-b");
        let b = source_from_command(&config, second, &command);
        assert_ne!(a.key(), b.key());
        assert!(!a.same_store(&b));
        assert!(!a.credential_source.contains("fixture-credential"));
        command.env_remove(name);
    }
}

#[test]
fn quota_snapshot_uses_percent_units_and_iso_resets() {
    let windows = limits(
        &json!({"five_hour":{"utilization":0.5,"resets_at":"2026-11-17T10:00:00Z"},"seven_day":{"utilization":60},"seven_day_sonnet":null}),
        10,
    );
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].remaining_percent, Some(99.5));
    assert!(windows[0].resets_at.is_some());
    assert_eq!(windows[1].duration_seconds, Some(604800));
}

#[test]
fn model_windows_and_extra_usage_keep_separate_units() {
    let windows = limits(
        &json!({"model_scoped":[{"display_name":"Model A","utilization":75}],"extra_usage":{"is_enabled":true,"used_credits":20,"monthly_limit":100,"currency":"USD"}}),
        100,
    );
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].key, "model:Model A");
    assert_eq!(windows[0].duration_seconds, Some(604800));
    assert_eq!(
        windows[1].value,
        Some(AccountQuotaValue::Usage {
            used: 20.,
            limit: Some(100.),
            unit: "credits".into()
        })
    );
}

#[test]
fn native_limit_events_use_fraction_units_and_keep_model_windows_separate() {
    let windows = event_limits(
        &json!({"rateLimitType":"seven_day_sonnet", "utilization":0.32, "resetsAt":1788766092}),
        100,
    );
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].key, "seven_day_sonnet");
    assert_eq!(windows[0].remaining_percent, Some(68.));
    assert_eq!(windows[0].duration_seconds, Some(604800));
    assert_eq!(windows[0].resets_at, Some(1788766092));
    assert!(
        event_limits(
            &json!({"rateLimitType":"seven_day", "status":"allowed"}),
            100
        )
        .is_empty()
    );
}
