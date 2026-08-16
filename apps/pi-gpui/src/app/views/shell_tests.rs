use std::{path::PathBuf, time::SystemTime};

use super::{
    SessionRailKind, compact_number, compact_subagent_label, context_usage, format_cache_hit_rate,
    format_cost, normalized_agent_status, session_badge, session_rail_items,
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
fn session_groups_mark_only_boundaries_after_a_preceding_group() {
    let sessions = vec![
        session("settled-one", true),
        session("active-one", false),
        session("settled-two", true),
        session("active-two", false),
    ];

    let all_groups = session_rail_items(&sessions, true);
    assert_eq!(all_groups[0].kind, SessionRailKind::Project);
    assert!(all_groups[0].starts_section);
    assert!(!all_groups[1].starts_section);
    assert_eq!(all_groups[2].kind, SessionRailKind::Settled);
    assert!(all_groups[2].starts_section);
    assert!(!all_groups[3].starts_section);

    let without_drafts = session_rail_items(&sessions, false);
    assert!(!without_drafts[0].starts_section);
    assert!(without_drafts[2].starts_section);

    let settled_only = session_rail_items(&sessions[..1], false);
    assert!(!settled_only[0].starts_section);

    let draft_then_settled = session_rail_items(&sessions[..1], true);
    assert!(draft_then_settled[0].starts_section);
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
