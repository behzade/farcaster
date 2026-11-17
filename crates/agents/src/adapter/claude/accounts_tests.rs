use super::*;

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
