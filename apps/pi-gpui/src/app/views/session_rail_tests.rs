use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::{
    ActiveRailRow, ProjectGroup, SessionRailItem, SessionRailKind, is_meaningful_active_status,
    minimal_row_splice, new_session_project, recent_archived_sessions, resolved_active_projects,
    roots_waiting_for_descendants, session_accessible_label, session_badge, session_rail_groups,
    visible_session_shortcuts,
};
use crate::{
    composer_sessions::draft_target,
    projects::DraftSession,
    sessions::{SessionSummary, UsageSummary},
};

#[test]
fn shortcuts_follow_visible_session_position_and_skip_headings() {
    let rows = vec![
        ActiveRailRow::Project(PathBuf::from("/project"), false),
        ActiveRailRow::Session(item("first", "/project", SessionRailKind::Project, false)),
        ActiveRailRow::Project(PathBuf::from("/other"), false),
        ActiveRailRow::Session(item("second", "/other", SessionRailKind::Project, false)),
    ];

    let shortcuts = visible_session_shortcuts(&rows);
    assert_eq!(shortcuts.get("first"), Some(&1));
    assert_eq!(shortcuts.get("second"), Some(&2));
}

#[test]
fn active_sessions_always_have_a_meaningful_state() {
    let done = item("done", "/project", SessionRailKind::Project, false);
    let running = item("running", "/project", SessionRailKind::Project, true);

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
fn archived_sessions_suppress_done_but_keep_active_states() {
    let archived = item("archived", "/project", SessionRailKind::Settled, false);
    let running = item("running", "/project", SessionRailKind::Settled, true);

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
    let parent = item("parent", "/project", SessionRailKind::Project, false);
    let mut child = item("child", "/project", SessionRailKind::Project, true).session;
    child.parent_session = Some(parent.session.id.clone());
    let waiting = roots_waiting_for_descendants(&[parent.session.clone(), child]);

    assert!(waiting.contains("parent"));
    assert_eq!(
        session_badge(&parent, Some("Done"), None, "", true),
        Some("Waiting".into())
    );
}

#[test]
fn runtime_activity_does_not_reorder_draft_and_session_projects() {
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
            inactive_draft_project.as_path(),
            runtime_draft_project.as_path(),
            live_project.as_path(),
            discovered_project.as_path(),
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
fn collapsed_archive_preview_keeps_the_three_most_recent_sessions() {
    let mut oldest = item("oldest", "/one", SessionRailKind::Settled, false);
    oldest.session.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    let mut second = item("second", "/one", SessionRailKind::Settled, false);
    second.session.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
    let mut third = item("third", "/two", SessionRailKind::Settled, false);
    third.session.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(3);
    let mut newest = item("newest", "/two", SessionRailKind::Settled, false);
    newest.session.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(4);
    let groups = vec![
        ProjectGroup {
            project: PathBuf::from("/one"),
            items: vec![oldest, second],
        },
        ProjectGroup {
            project: PathBuf::from("/two"),
            items: vec![third, newest],
        },
    ];

    let preview = recent_archived_sessions(&groups, 3);
    assert_eq!(
        preview
            .iter()
            .map(|item| item.session.id.as_str())
            .collect::<Vec<_>>(),
        ["newest", "third", "second"]
    );
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
fn new_session_uses_viewed_chat_project_independent_of_filter() {
    let current = Path::new("/current");
    assert_eq!(new_session_project(current, None), current);
    assert_eq!(
        new_session_project(current, Some(Path::new("/filtered"))),
        PathBuf::from("/current")
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
