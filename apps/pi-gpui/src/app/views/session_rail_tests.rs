use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::{
    SessionRailItem, SessionRailKind, is_meaningful_active_status, minimal_row_splice,
    resolved_active_projects, selected_new_session_project, session_accessible_label,
    session_badge, session_rail_groups,
};
use crate::{
    composer_sessions::draft_target,
    projects::DraftSession,
    sessions::{SessionSummary, UsageSummary},
};

#[test]
fn active_sessions_always_have_a_meaningful_state() {
    let done = item("done", "/project", SessionRailKind::Project, false);
    let running = item("running", "/project", SessionRailKind::Project, true);

    assert_eq!(
        session_badge(&done, None, Some("other"), "Working"),
        Some("Done".into())
    );
    assert_eq!(
        session_badge(&done, Some("Ready"), None, ""),
        Some("Done".into())
    );
    assert_eq!(
        session_badge(&running, None, None, ""),
        Some("Working".into())
    );
    assert_eq!(
        session_badge(&done, None, Some("done"), "Needs input"),
        Some("Needs input".into())
    );
}

#[test]
fn archived_sessions_suppress_done_but_keep_active_states() {
    let archived = item("archived", "/project", SessionRailKind::Settled, false);
    let running = item("running", "/project", SessionRailKind::Settled, true);

    assert_eq!(session_badge(&archived, Some("Done"), None, ""), None);
    assert_eq!(session_badge(&archived, None, None, ""), None);
    assert_eq!(
        session_badge(&running, None, None, ""),
        Some("Working".into())
    );
}

#[test]
fn resolved_ui_activity_prioritizes_projects_stably() {
    let inactive_draft_project = PathBuf::from("/inactive-draft");
    let runtime_draft_project = PathBuf::from("/runtime-draft");
    let live_project = PathBuf::from("/live");
    let discovered_project = PathBuf::from("/discovered");
    let inactive_session_project = PathBuf::from("/inactive-session");
    let drafts = vec![
        DraftSession::with_id("inactive-draft".into(), inactive_draft_project.clone()),
        DraftSession::with_id("runtime-draft".into(), runtime_draft_project.clone()),
    ];
    let sessions = vec![
        item("live", "/live", SessionRailKind::Project, false).session,
        item("discovered", "/discovered", SessionRailKind::Project, true).session,
        item(
            "inactive-session",
            "/inactive-session",
            SessionRailKind::Project,
            false,
        )
        .session,
    ];
    let statuses = HashMap::from([(draft_target("runtime-draft"), "Retrying".into())]);

    let active_projects = resolved_active_projects(
        &sessions,
        &drafts,
        &HashMap::new(),
        &statuses,
        Some("live"),
        "Compacting",
        None,
    );
    let groups = session_rail_groups(
        &sessions,
        &drafts,
        &[
            "live".into(),
            "discovered".into(),
            "inactive-session".into(),
        ],
        None,
        &active_projects,
    );
    let projects = groups
        .active
        .iter()
        .map(|group| group.project.as_path())
        .collect::<Vec<_>>();

    assert_eq!(
        projects,
        vec![
            runtime_draft_project.as_path(),
            live_project.as_path(),
            discovered_project.as_path(),
            inactive_draft_project.as_path(),
            inactive_session_project.as_path(),
        ]
    );
}

#[test]
fn only_meaningful_runtime_states_mark_a_project_active() {
    for status in ["Working", "Needs input", "Compacting", "Retrying"] {
        assert!(is_meaningful_active_status(status));
    }
    for status in ["", "Draft", "Done", "Ready", "Idle"] {
        assert!(!is_meaningful_active_status(status));
    }
}

#[test]
fn minimal_row_reconciliation_preserves_equal_prefix_and_suffix() {
    let current = vec!["project", "one", "two", "three"];

    assert_eq!(minimal_row_splice(&current, &current), None);
    assert_eq!(
        minimal_row_splice(&current, &["project", "one", "changed", "three"]),
        Some((2..3, 1))
    );
    assert_eq!(
        minimal_row_splice(&current, &["project", "one", "two", "three", "four"]),
        Some((4..4, 1))
    );
    assert_eq!(
        minimal_row_splice(&current, &["project", "three"]),
        Some((1..3, 0))
    );
}

#[test]
fn selected_project_controls_new_session_destination() {
    let current = Path::new("/current");
    assert_eq!(selected_new_session_project(None, current), current);
    assert_eq!(
        selected_new_session_project(Some(Path::new("/filtered")), current),
        PathBuf::from("/filtered")
    );
}

#[test]
fn session_accessible_name_contains_state_and_relative_time() {
    assert_eq!(
        session_accessible_label("Fix grouping", "Working", "2m"),
        "Resume session: Fix grouping. State: Working. Updated 2m"
    );
}

fn item(id: &str, project: &str, kind: SessionRailKind, is_running: bool) -> SessionRailItem {
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
            kind == SessionRailKind::Settled,
            is_running,
            String::new(),
        ),
        kind,
    }
}
