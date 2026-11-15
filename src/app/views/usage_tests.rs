use super::*;

#[test]
fn working_context_hides_transient_zeroes() {
    let zero = serde_json::json!({
        "contextUsage": {"tokens": 0, "contextWindow": 200_000, "percent": 0.0}
    });
    let known = serde_json::json!({
        "contextUsage": {"tokens": 168_000, "contextWindow": 200_000, "percent": 84.0}
    });

    assert_eq!(visible_context_stats(&zero, true), None);
    assert_eq!(visible_context_stats(&zero, false), Some(&zero));
    assert_eq!(visible_context_stats(&known, true), Some(&known));
}

#[test]
fn context_projection_handles_explicit_derived_and_partial_values() {
    let explicit = context_summary(Some(&serde_json::json!({
        "contextUsage": {"tokens": 160_000, "contextWindow": 200_000, "percent": 81.25}
    })));
    assert_eq!(explicit.percent, Some(81.25));
    assert_eq!(explicit.used, Some(160_000));
    assert_eq!(explicit.total, Some(200_000));

    let derived = context_summary(Some(&serde_json::json!({
        "contextUsage": {"tokens": 50, "contextWindow": 200}
    })));
    assert_eq!(derived.percent, Some(25.0));

    let partial = context_summary(Some(&serde_json::json!({
        "contextUsage": {"tokens": 25_000}
    })));
    assert_eq!(partial.percent, None);
    assert_eq!(partial.total, None);
}

#[test]
fn footer_values_use_compact_stable_formatting() {
    assert_eq!(format_tokens(320), "320");
    assert_eq!(format_tokens(1_200), "1.2k");
    assert_eq!(format_tokens(14_600), "14.6k");
    assert_eq!(format_tokens(128_000), "128k");
    assert_eq!(format_tokens(2_000_000), "2.0M");
    assert_eq!(format_cost(18_000), "$0.018");
}

#[test]
fn worker_usage_requires_reported_tokens_or_cost() {
    assert!(!has_reported_usage(&UsageSummary::default()));
    for usage in [
        UsageSummary {
            input: 1,
            ..Default::default()
        },
        UsageSummary {
            output: 1,
            ..Default::default()
        },
        UsageSummary {
            cache_read: 1,
            ..Default::default()
        },
        UsageSummary {
            cache_write: 1,
            ..Default::default()
        },
        UsageSummary {
            total: 1,
            ..Default::default()
        },
        UsageSummary {
            cost_micros: 1,
            ..Default::default()
        },
    ] {
        assert!(has_reported_usage(&usage));
    }
}
