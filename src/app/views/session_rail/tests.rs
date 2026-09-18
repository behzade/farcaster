use crate::agents::Backend;
use std::{path::PathBuf, time::SystemTime};

use super::{
    ARCHIVED_LEADING_GAP, ActiveSessionItem, SessionRailItem, SessionRailKind,
    clamped_session_rail_width, collapsed_inactive_rail_height, first_unsubmitted_draft,
    hover::session_tooltip_lines, minimal_row_splice, numbered_session_items,
    replacement_index_after_close, session_accessible_label, status_visual, subagent_counts,
};
use crate::{
    app::session_folders::{SessionFolder, SessionFolders},
    app::ui::assets::AppIcon,
    app::ui::theme::theme,
    sessions::{DraftSession, SessionSummary, UsageSummary},
};

#[test]
fn closing_a_session_keeps_its_visual_slot_when_possible() {
    assert_eq!(replacement_index_after_close(4, 1), Some(2));
    assert_eq!(replacement_index_after_close(4, 3), Some(2));
    assert_eq!(replacement_index_after_close(1, 0), None);
}

#[test]
fn numbers_address_the_chats_of_one_folder_at_a_time() {
    let mut first_draft =
        DraftSession::with_id(Some(Backend::Pi), "first".into(), PathBuf::from("/project"));
    first_draft.app_session_id = 12;
    let mut second_draft = DraftSession::with_id(
        Some(Backend::Pi),
        "second".into(),
        PathBuf::from("/project"),
    );
    second_draft.app_session_id = 11;
    let mut submitted = DraftSession::with_id(
        Some(Backend::Pi),
        "submitted".into(),
        PathBuf::from("/project"),
    );
    submitted.app_session_id = 10;
    submitted.submitted = true;
    let persisted = item("persisted", 9, "/other", SessionRailKind::Project, false);
    let filed = item("filed", 7, "/other", SessionRailKind::Project, false);
    let rows = vec![
        ActiveSessionItem::Draft(first_draft),
        ActiveSessionItem::Draft(second_draft),
        ActiveSessionItem::Draft(submitted),
        ActiveSessionItem::Session(persisted),
        ActiveSessionItem::Session(filed),
    ];
    let mut folders = SessionFolders {
        folders: vec![
            SessionFolder {
                id: 1,
                name: "Project".into(),
                ..Default::default()
            },
            SessionFolder {
                id: 2,
                name: "Later".into(),
                ..Default::default()
            },
            SessionFolder {
                id: 3,
                name: "Empty".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    folders.assign(12, Some(1));
    folders.assign(10, Some(1));
    folders.assign(9, Some(2));
    folders.assign(7, Some(1));

    let all = numbered_session_items(&rows, &folders, None);
    let first_only = numbered_session_items(&rows, &folders, Some(1));
    let second_only = numbered_session_items(&rows, &folders, Some(2));
    let empty_only = numbered_session_items(&rows, &folders, Some(3));

    assert_eq!(
        first_unsubmitted_draft(&rows).map(|draft| draft.id.as_str()),
        Some("first")
    );
    assert_eq!(
        all.iter()
            .map(|(id, item)| (*id, item.app_session_id()))
            .collect::<Vec<_>>(),
        [(1, 12), (1, 10), (1, 7), (2, 9)]
    );
    assert_eq!(
        first_only
            .iter()
            .map(|(_, item)| item.app_session_id())
            .collect::<Vec<_>>(),
        [12, 10, 7]
    );
    assert_eq!(
        second_only
            .iter()
            .map(|(_, item)| item.app_session_id())
            .collect::<Vec<_>>(),
        [9]
    );
    assert!(empty_only.is_empty());
}

#[test]
fn session_numbers_stop_at_ten_so_zero_reaches_the_tenth_chat() {
    let rows = (0..12)
        .map(|index| {
            ActiveSessionItem::Session(item(
                &format!("chat-{index}"),
                i64::from(index) + 1,
                "/project",
                SessionRailKind::Project,
                false,
            ))
        })
        .collect::<Vec<_>>();
    let folders = SessionFolders {
        folders: vec![SessionFolder {
            id: 1,
            name: "Project".into(),
            project: Some(PathBuf::from("/project")),
            ..Default::default()
        }],
        ..Default::default()
    };

    let numbered = numbered_session_items(&rows, &folders, None);

    assert_eq!(numbered.len(), 10);
    assert_eq!(numbered[9].1.app_session_id(), 10);
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
fn collapsed_archived_rail_includes_a_leading_gap() {
    let without_gap = collapsed_inactive_rail_height(2, false);
    let archived = collapsed_inactive_rail_height(2, true);
    assert_eq!(
        f32::from(without_gap),
        f32::from(theme().controls.utility_row)
            + f32::from(theme().controls.archived_preview_row) * 2.0
    );
    assert_eq!(
        f32::from(archived) - f32::from(without_gap),
        ARCHIVED_LEADING_GAP
    );
}

#[test]
fn collapsed_archived_rail_previews_at_most_five_sessions() {
    let height = collapsed_inactive_rail_height(10, false);
    assert_eq!(
        f32::from(height),
        f32::from(theme().controls.utility_row)
            + f32::from(theme().controls.archived_preview_row) * 5.0
    );
}

#[test]
fn session_rail_resize_stays_within_design_bounds() {
    assert_eq!(
        clamped_session_rail_width(100.0),
        theme().layout.session_rail_min
    );
    assert_eq!(clamped_session_rail_width(286.0), theme().size(286.0));
    assert_eq!(
        clamped_session_rail_width(500.0),
        theme().layout.session_rail_max
    );
}

#[test]
fn session_states_use_semantic_icons() {
    assert_eq!(
        status_visual("Done").map(|(icon, _)| icon),
        Some(AppIcon::CheckCircle)
    );
    assert_eq!(
        status_visual("Complete").map(|(icon, _)| icon),
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
        status_visual("Incomplete").map(|(icon, _)| icon),
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
        .with_app_session_id(app_session_id),
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
