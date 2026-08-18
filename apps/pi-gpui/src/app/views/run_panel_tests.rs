use super::*;

#[test]
fn working_context_never_flashes_a_false_zero() {
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
fn context_projection_prefers_explicit_percent_and_derives_remaining() {
    let summary = context_summary(
        Some(&serde_json::json!({
            "contextUsage": {"tokens": 160_000, "contextWindow": 200_000, "percent": 81.25}
        })),
        456_789,
    );
    assert_eq!(summary.percent, Some(81.25));
    assert_eq!(summary.remaining, Some(40_000));
    assert_eq!(summary.cost_micros, 456_789);
    assert!(summary.warning);
    assert_eq!(compact_number(summary.used.unwrap_or_default()), "160k");
    assert_eq!(format_cost(summary.cost_micros), "$0.46");
}

#[test]
fn context_projection_handles_partial_missing_and_zero_windows() {
    let partial = context_summary(
        Some(&serde_json::json!({"contextUsage": {"tokens": 25_000}})),
        0,
    );
    assert_eq!(partial.used, Some(25_000));
    assert_eq!(partial.total, None);
    assert_eq!(partial.percent, None);
    assert_eq!(partial.remaining, None);
    assert!(!partial.warning);

    let zero = context_summary(
        Some(&serde_json::json!({
            "contextUsage": {"tokens": 1, "contextWindow": 0}
        })),
        0,
    );
    assert_eq!(zero.percent, None);
    assert_eq!(context_summary(None, 0).used, None);
}

#[test]
fn context_projection_derives_percent_when_rpc_omits_it() {
    let summary = context_summary(
        Some(&serde_json::json!({
            "contextUsage": {"tokens": 50, "contextWindow": 200}
        })),
        0,
    );
    assert_eq!(summary.percent, Some(25.0));
    assert_eq!(summary.remaining, Some(150));
}

#[test]
fn duration_and_lifecycle_labels_are_truthful() {
    assert_eq!(format_duration(Some(Duration::from_secs(65))), "1m 5s");
    assert_eq!(
        lifecycle_label(AgentLifecycle::Completed(AgentOutcome::Failed)),
        "Failed"
    );
    assert_eq!(lifecycle_label(AgentLifecycle::Unknown), "Unknown");
}

#[test]
fn every_lifecycle_has_a_compact_status_icon() {
    assert_eq!(lifecycle_icon(AgentLifecycle::Working), AppIcon::SpinnerGap);
    assert_eq!(
        lifecycle_icon(AgentLifecycle::NeedsInput),
        AppIcon::WarningCircle
    );
    assert_eq!(lifecycle_icon(AgentLifecycle::Unknown), AppIcon::Question);
    assert_eq!(
        lifecycle_icon(AgentLifecycle::Completed(AgentOutcome::Complete)),
        AppIcon::CheckCircle
    );
    assert_eq!(
        lifecycle_icon(AgentLifecycle::Completed(AgentOutcome::Failed)),
        AppIcon::XCircle
    );
    assert_eq!(
        lifecycle_icon(AgentLifecycle::Completed(AgentOutcome::Incomplete)),
        AppIcon::WarningCircle
    );
}

#[test]
fn active_agents_are_never_hidden_by_limited_history() {
    assert_eq!(
        agent_section(AgentLifecycle::Working, true, true),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::NeedsInput, true, true),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::Unknown, true, false),
        AgentSection::Limited
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            true,
            false
        ),
        AgentSection::Limited
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            false,
            false
        ),
        AgentSection::Completed
    );
}
