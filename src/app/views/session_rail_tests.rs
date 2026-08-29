use std::{path::PathBuf, time::SystemTime};

use super::{
    ActiveSessionItem, SessionRailItem, SessionRailKind, collapsed_inactive_rail_height,
    first_unsubmitted_draft, minimal_row_splice, roots_waiting_for_descendants,
    session_accessible_label, session_badge, status_visual, subagent_counts,
    visible_session_shortcuts,
};
use crate::{
    app::views::session_hover::session_tooltip_lines,
    assets::AppIcon,
    projects::DraftSession,
    sessions::{SessionSummary, UsageSummary},
    theme::THEME,
};

#[test]
fn shortcuts_reserve_zero_for_the_first_unsubmitted_draft() {
    let mut first_draft = DraftSession::with_id("first".into(), PathBuf::from("/project"));
    first_draft.app_session_id = 12;
    let mut second_draft = DraftSession::with_id("second".into(), PathBuf::from("/project"));
    second_draft.app_session_id = 11;
    let mut submitted = DraftSession::with_id("submitted".into(), PathBuf::from("/project"));
    submitted.app_session_id = 10;
    submitted.submitted = true;
    let persisted = item("persisted", 9, "/other", SessionRailKind::Project, false);
    let rows = vec![
        ActiveSessionItem::Draft(first_draft),
        ActiveSessionItem::Draft(second_draft),
        ActiveSessionItem::Draft(submitted),
        ActiveSessionItem::Session(persisted),
    ];

    let shortcuts = visible_session_shortcuts(&rows);

    assert_eq!(
        first_unsubmitted_draft(&rows).map(|draft| draft.id.as_str()),
        Some("first")
    );
    assert_eq!(shortcuts.get(&12), Some(&0));
    assert!(!shortcuts.contains_key(&11));
    assert_eq!(shortcuts.get(&10), Some(&1));
    assert_eq!(shortcuts.get(&9), Some(&2));
}

#[test]
fn active_sessions_always_have_a_meaningful_state() {
    let done = item("done", 2, "/project", SessionRailKind::Project, false);
    let running = item("running", 1, "/project", SessionRailKind::Project, true);

    assert_eq!(
        session_badge(&done, None, Some("other"), "Working", false),
        Some("Done".into())
    );
    assert_eq!(
        session_badge(&done, Some("Ready"), None, "", false),
        Some("Done".into())
    );
    assert_eq!(
        session_badge(&running, None, None, "", false),
        Some("Working".into())
    );
    assert_eq!(
        session_badge(&done, None, Some("done"), "Needs input", false),
        Some("Needs input".into())
    );
}

#[test]
fn review_and_archived_sessions_suppress_done_but_keep_active_states() {
    let review = item("review", 3, "/project", SessionRailKind::Review, false);
    let archived = item("archived", 2, "/project", SessionRailKind::Archived, false);
    let running = item("running", 1, "/project", SessionRailKind::Archived, true);

    assert_eq!(session_badge(&review, Some("Done"), None, "", false), None);
    assert_eq!(
        session_badge(&archived, Some("Done"), None, "", false),
        None
    );
    assert_eq!(session_badge(&archived, None, None, "", false), None);
    assert_eq!(
        session_badge(&running, None, None, "", false),
        Some("Working".into())
    );
}

#[test]
fn completed_parent_waits_while_a_descendant_is_running() {
    let parent = item("parent", 2, "/project", SessionRailKind::Project, false);
    let mut child = item("child", 1, "/project", SessionRailKind::Project, true).session;
    child.parent_session = Some(parent.session.id.clone());
    let waiting = roots_waiting_for_descendants(&[parent.session.clone(), child]);

    assert!(waiting.contains("parent"));
    assert_eq!(
        session_badge(&parent, Some("Done"), None, "", true),
        Some("Waiting".into())
    );
}

#[test]
fn minimal_row_reconciliation_preserves_equal_prefix_and_suffix() {
    let current = vec!["one", "two", "three"];

    assert_eq!(minimal_row_splice(&current, &current), None);
    assert_eq!(
        minimal_row_splice(&current, &["one", "changed", "three"]),
        Some((1..2, 1))
    );
    assert_eq!(
        minimal_row_splice(&current, &["one", "two", "three", "four"]),
        Some((3..3, 1))
    );
    assert_eq!(minimal_row_splice(&current, &["three"]), Some((0..2, 0)));
}

#[test]
fn collapsed_archived_rail_leaves_space_after_review() {
    let review = collapsed_inactive_rail_height(2, false);
    let archived = collapsed_inactive_rail_height(2, true);
    assert_eq!(
        f32::from(review),
        f32::from(THEME.controls.utility_row)
            + f32::from(THEME.controls.archived_preview_row) * 2.0
    );
    assert_eq!(
        f32::from(archived) - f32::from(review),
        f32::from(THEME.space.md)
    );
}

#[test]
fn session_states_use_semantic_icons() {
    assert_eq!(
        status_visual("Done").map(|(icon, _)| icon),
        Some(AppIcon::CheckCircle)
    );
    assert_eq!(
        status_visual("Working").map(|(icon, _)| icon),
        Some(AppIcon::SpinnerGap)
    );
    assert_eq!(
        status_visual("Needs input").map(|(icon, _)| icon),
        Some(AppIcon::WarningCircle)
    );
    assert_eq!(
        status_visual("Waiting").map(|(icon, _)| icon),
        Some(AppIcon::Hourglass)
    );
    assert_eq!(status_visual("").map(|(icon, _)| icon), None);
}

#[test]
fn session_accessible_name_contains_state_and_relative_time() {
    assert_eq!(
        session_accessible_label("Fix grouping", "Working", "2m"),
        "Resume session: Fix grouping. State: Working. Updated 2m"
    );
}

fn item(
    id: &str,
    app_session_id: i64,
    project: &str,
    kind: SessionRailKind,
    is_running: bool,
) -> SessionRailItem {
    SessionRailItem {
        session: SessionSummary::from_cached(
            id.into(),
            PathBuf::from(format!("/{id}.jsonl")),
            PathBuf::from(project),
            id.into(),
            String::new(),
            String::new(),
            None,
            if is_running {
                SystemTime::now()
            } else {
                SystemTime::UNIX_EPOCH
            },
            0,
            UsageSummary::default(),
            kind == SessionRailKind::Archived,
            is_running,
            String::new(),
        )
        .with_app_session_id(app_session_id)
        .with_review(kind == SessionRailKind::Review),
        kind,
    }
}

#[test]
fn tooltips_report_model_effort_and_direct_subagent_counts() {
    let mut modelled = item("modelled", 1, "/project", SessionRailKind::Project, false);
    modelled.session.model = Some(("anthropic".into(), "claude-opus-4-5".into()));
    modelled.session.thinking_level = Some("high".into());
    let lines = session_tooltip_lines(&modelled.session, 1);
    assert!(
        lines
            .iter()
            .any(|line| line == "Model: anthropic / claude-opus-4-5")
    );
    assert!(lines.iter().any(|line| line == "Effort: High"));
    assert!(lines.iter().any(|line| line == "Subagents: 1 subagent"));

    let mut parent = item("parent", 2, "/project", SessionRailKind::Project, false);
    parent.session.parent_session = Some("root".into());
    let mut other = item("other", 3, "/project", SessionRailKind::Project, false);
    other.session.parent_session = Some("root".into());
    let sessions = vec![
        item("root", 0, "/project", SessionRailKind::Project, false).session,
        parent.session,
        other.session,
    ];

    let counts = subagent_counts(&sessions);

    assert_eq!(counts.get("root"), Some(&2));
    assert!(
        session_tooltip_lines(
            sessions.first().expect("root session fixture"),
            counts["root"],
        )
        .iter()
        .any(|line| line == "Subagents: 2 subagents")
    );
}
