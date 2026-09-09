use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use super::*;
use crate::sessions::UsageSummary;

#[test]
fn drafts_and_sessions_share_one_descending_id_order() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let mut draft = DraftSession::with_id("draft".into(), alpha.clone());
    draft.app_session_id = 2;
    let sessions = vec![
        session("old", 1, &alpha, false),
        session("new", 3, &beta, false),
    ];

    let lists = session_rail_lists(&sessions, &[draft], None, &[]);

    assert_eq!(
        lists
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [3, 2, 1]
    );
}

#[test]
fn manual_order_overrides_id_order_and_new_ids_stay_first() {
    let project = PathBuf::from("/project");
    let sessions = vec![
        session("new", 4, &project, false),
        session("three", 3, &project, false),
        session("two", 2, &project, false),
        session("one", 1, &project, false),
    ];

    let lists = session_rail_lists(&sessions, &[], None, &[1, 3, 2]);

    assert_eq!(
        lists
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [4, 1, 3, 2]
    );
}

#[test]
fn reorder_uses_before_and_after_insertion_gaps() {
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 4, 2, ReorderPosition::After),
        Some(vec![3, 2, 4, 1])
    );
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 4, 1, ReorderPosition::After),
        Some(vec![3, 2, 1, 4])
    );
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 1, 3, ReorderPosition::Before),
        Some(vec![4, 1, 3, 2])
    );
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 3, 3, ReorderPosition::Before),
        None
    );
}

#[test]
fn filtered_reorder_preserves_hidden_row_positions() {
    assert_eq!(
        merge_visible_session_order(&[5, 4, 3, 2, 1], &[1, 3, 5]),
        [1, 4, 3, 2, 5]
    );
}

#[test]
fn project_filter_keeps_a_flat_subset() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let mut alpha_draft = DraftSession::with_id("alpha-draft".into(), alpha.clone());
    alpha_draft.app_session_id = 4;
    let mut beta_draft = DraftSession::with_id("beta-draft".into(), beta.clone());
    beta_draft.app_session_id = 3;
    let sessions = vec![
        session("alpha-active", 2, &alpha, false),
        session("beta-archived", 1, &beta, true),
    ];

    let lists = session_rail_lists(
        &sessions,
        &[alpha_draft, beta_draft],
        Some(beta.as_path()),
        &[],
    );

    assert_eq!(lists.active.len(), 1);
    assert_eq!(lists.active[0].app_session_id(), 3);
    assert_eq!(lists.archived.len(), 1);
    assert_eq!(lists.archived[0].session.app_session_id, 1);
}

#[test]
fn archived_sessions_are_sorted_by_recency_not_imported_id() {
    let project = PathBuf::from("/project");
    let mut recent = session("recent", 1, &project, true);
    recent.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
    let mut old = session("old", 2, &project, true);
    old.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1);

    let lists = session_rail_lists(&[old, recent], &[], None, &[]);

    assert_eq!(
        lists
            .archived
            .iter()
            .map(|item| item.session.id.as_str())
            .collect::<Vec<_>>(),
        ["recent", "old"]
    );
}

#[test]
fn promotion_identity_is_rendered_once_and_prefers_the_draft() {
    let project = PathBuf::from("/project");
    let mut draft = DraftSession::with_id("draft".into(), project.clone());
    draft.app_session_id = 7;
    draft.submitted = true;
    let persisted = session("persisted", 7, &project, false);

    let lists = session_rail_lists(&[persisted], &[draft], None, &[]);

    assert_eq!(lists.active.len(), 1);
    assert!(matches!(lists.active[0], ActiveSessionItem::Draft(_)));
}

#[test]
fn unassigned_fallback_sessions_are_not_deduplicated() {
    let project = PathBuf::from("/project");
    let sessions = vec![
        session("one", 0, &project, false),
        session("two", 0, &project, false),
    ];

    let lists = session_rail_lists(&sessions, &[], None, &[]);

    assert_eq!(lists.active.len(), 2);
}

fn session(id: &str, app_session_id: i64, project: &Path, archived: bool) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/{id}.jsonl")),
        project.to_path_buf(),
        id.into(),
        String::new(),
        String::new(),
        None,
        SystemTime::UNIX_EPOCH,
        0,
        UsageSummary::default(),
        archived,
        false,
        String::new(),
    )
    .with_app_session_id(app_session_id)
}
