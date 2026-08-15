use super::{
    SessionRailKind, compact_number, compact_subagent_label, context_usage, format_cache_hit_rate,
    format_cost, session_badge,
};

#[test]
fn subagent_labels_keep_the_role_and_drop_generated_ids() {
    assert_eq!(
        compact_subagent_label("subagent-reviewer-a7d59830-87da-46d7-1"),
        "reviewer 1"
    );
    assert_eq!(compact_subagent_label("named child"), "named child");
}

#[test]
fn usage_values_are_compact_and_context_is_main_only() {
    assert_eq!(compact_number(105_250), "105.2k");
    assert_eq!(format_cache_hit_rate(Some(87.654)), "87.7%");
    assert_eq!(format_cache_hit_rate(None), "—");
    assert_eq!(format_cost(456_789), "$0.46");
    assert_eq!(
        context_usage(&serde_json::json!({
            "contextUsage": {"tokens": 60_000, "contextWindow": 200_000, "percent": 30.0}
        })),
        "60k / 200k · 30%"
    );
}

#[test]
fn session_badges_show_only_meaningful_live_state() {
    assert_eq!(
        session_badge(SessionRailKind::Project, "other", Some("live"), "Working"),
        None
    );
    assert_eq!(
        session_badge(SessionRailKind::Settled, "live", Some("live"), "Working"),
        Some("Working".into())
    );
    assert_eq!(
        session_badge(SessionRailKind::Project, "live", Some("live"), "Done"),
        None
    );
}
