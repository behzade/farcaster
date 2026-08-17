use std::{path::PathBuf, time::SystemTime};

use super::{
    SessionRailKind, archived_run_badge, compact_number, compact_subagent_label, context_usage,
    format_cache_hit_rate, format_cost, normalized_agent_status, session_badge, session_rail_items,
};
use crate::sessions::{SessionSummary, UsageSummary};

#[test]
fn subagent_labels_keep_the_role_and_drop_generated_ids() {
    assert_eq!(
        compact_subagent_label("subagent-reviewer-a7d59830-87da-46d7-1"),
        "reviewer 1"
    );
    assert_eq!(compact_subagent_label("named child"), "named child");
}

#[test]
fn agent_status_keeps_live_work_visible_and_normalizes_idle_states() {
    assert_eq!(normalized_agent_status("Working"), "Working");
    assert_eq!(normalized_agent_status("Needs input"), "Needs input");
    assert_eq!(normalized_agent_status("Ready"), "Done");
    assert_eq!(normalized_agent_status(""), "Done");
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

#[test]
fn archived_session_badges_hide_completed_run_status() {
    assert_eq!(archived_run_badge(Some("Done")), None);
    assert_eq!(archived_run_badge(Some("Working")), Some("Working".into()));
}

#[test]
fn session_groups_keep_active_and_archived_roots_separate() {
    let sessions = vec![
        session("settled-one", true),
        session("active-one", false),
        session("settled-two", true),
        session("active-two", false),
    ];

    let groups = session_rail_items(
        &sessions,
        &[
            "active-two".into(),
            "settled-two".into(),
            "active-one".into(),
            "settled-one".into(),
        ],
    );
    assert_eq!(groups.active.len(), 2);
    assert_eq!(groups.archived.len(), 2);
    assert!(
        groups
            .active
            .iter()
            .all(|item| item.kind == SessionRailKind::Project)
    );
    assert!(
        groups
            .archived
            .iter()
            .all(|item| item.kind == SessionRailKind::Settled)
    );
    assert_eq!(groups.active[0].session.id, "active-two");
    assert_eq!(groups.archived[0].session.id, "settled-two");
}

fn session(id: &str, settled: bool) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/{id}.jsonl")),
        PathBuf::from("/project"),
        id.into(),
        String::new(),
        String::new(),
        None,
        SystemTime::UNIX_EPOCH,
        0,
        UsageSummary::default(),
        settled,
        false,
        String::new(),
    )
}
