use super::*;

#[test]
fn native_buckets_keep_their_identity_and_window_durations() {
    let windows = limits(
        &json!({"rateLimitsByLimitId":{
            "codex":{"limitId":"codex","primary":{"usedPercent":10,"windowDurationMins":300},"secondary":{"usedPercent":70,"windowDurationMins":10080,"resetsAt":1234}},
            "review":{"primary":{"usedPercent":25,"windowDurationMins":10080}}
        }}),
        100,
    );
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[1].duration_seconds, Some(604800));
    assert_eq!(windows[1].remaining_percent, Some(30.));
    assert_ne!(windows[1].key, windows[2].key);
    assert_eq!(windows[1].resets_at, Some(1234));
}

#[test]
fn account_and_workspace_are_separate_and_api_keys_do_not_invent_principals() {
    let id = identity(
        &json!({"type":"chatgpt","userId":"user","accountId":"workspace","email":"me@example.com"}),
    )
    .expect("identity");
    assert_eq!(id.principal_id.as_deref(), Some("user"));
    assert_eq!(id.workspace_id.as_deref(), Some("workspace"));
    assert!(
        identity(&json!({"type":"apiKey"}))
            .expect("API connection")
            .principal_id
            .is_none()
    );
    assert!(identity(&Value::Null).is_none());
}

#[test]
fn native_credits_are_not_percentages_or_verified_pool_ids() {
    let windows = limits(
        &json!({"rateLimits":{"limitId":"codex","credits":{"balance":"23.50","unlimited":false},"primary":{"usedPercent":10}}}),
        100,
    );
    assert_eq!(windows.len(), 2);
    assert!(windows.iter().all(|w| w.pool_id.is_none()));
    assert_eq!(windows[1].remaining_percent, None);
    assert_eq!(
        windows[1].value,
        Some(AccountQuotaValue::Balance {
            remaining: 23.5,
            unit: "credits".into()
        })
    );
}

#[test]
fn native_limit_notifications_use_the_named_bucket_key() {
    let notification = json!({"rateLimits":{"limitId":"review", "secondary":{"usedPercent":42, "windowDurationMins":10080}}});
    let windows = limits(&notification, 100);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].key, "review:secondary");
    assert_eq!(windows[0].remaining_percent, Some(58.));
}
