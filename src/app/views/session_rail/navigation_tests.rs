use super::*;
use crate::agents::Backend;
use crate::app::views::session_rail::groups::session_rail_lists;

fn archived_session(item: &ActiveSessionItem) -> &SessionSummary {
    match item {
        ActiveSessionItem::Session(item) => &item.session,
        ActiveSessionItem::Draft(_) => panic!("expected an archived chat, found a draft"),
    }
}

#[test]
fn worker_shortcuts_wrap_through_parent_and_workers() {
    assert_eq!(worker_step(0, Some(0), 1), None);
    assert_eq!(worker_step(1, None, 1), Some(0));
    assert_eq!(worker_step(3, Some(0), 1), Some(1));
    assert_eq!(worker_step(3, Some(0), -1), Some(2));
    assert_eq!(worker_step(3, Some(2), 1), Some(0));
}

#[test]
fn worker_shortcuts_follow_the_visible_sidebar_view() {
    let recent = visible_worker_indices(9, Some(0), false);
    let older = visible_worker_indices(9, None, true);
    assert_eq!(recent.first(), Some(&0));
    assert_eq!(older, (0..9).collect::<Vec<_>>());

    let selected_older = older[5];
    assert_eq!(
        visible_worker_indices(9, Some(selected_older), false),
        recent
            .into_iter()
            .chain([selected_older])
            .collect::<Vec<_>>()
    );
    assert_eq!(visible_worker_indices(9, Some(selected_older), true), older);
}

#[test]
fn navigation_can_leave_and_return_to_an_unsubmitted_draft() {
    use crate::app::views::session_rail::folders;
    let drafts = [(30, false), (20, true), (10, false)].map(|(id, submitted)| {
        let mut draft = crate::sessions::DraftSession::with_id(
            Some(Backend::Pi),
            format!("draft-{id}"),
            "/project".into(),
        );
        draft.app_session_id = id;
        draft.submitted = submitted;
        draft
    });
    let rows = session_rail_lists(&[], &drafts, None, &[]).active;
    let shortcuts = crate::app::views::session_rail::numbered_session_items(&rows);
    assert_eq!(
        shortcuts
            .iter()
            .map(|item| item.app_session_id())
            .collect::<Vec<_>>(),
        [20]
    );
    let ids = folders::folder_rows(rows, &Default::default())
        .into_iter()
        .filter_map(VisibleSessionTarget::from_row)
        .map(|target| target.app_session_id())
        .collect::<Vec<_>>();
    assert_eq!(ids, [30, 20, 10]);
    for (selected, direction, expected) in [
        (30, 1, Some(SessionStep::Active(1))),
        (20, -1, Some(SessionStep::Active(0))),
        (10, -1, Some(SessionStep::Active(1))),
        (10, 1, Some(SessionStep::Archived(0))),
    ] {
        assert_eq!(
            session_step(ids.iter().copied(), [5], selected, direction),
            expected
        );
    }
}

#[test]
fn navigation_crosses_archive_preview_in_both_directions_without_wrapping() {
    let active = [10, 4];
    let archived = [30, 22, 19, 16, 15, 12, 8];
    for (selected, direction, expected) in [
        (10, -1, None),
        (10, 1, Some(SessionStep::Active(1))),
        (4, 1, Some(SessionStep::Archived(0))),
        (30, -1, Some(SessionStep::Active(1))),
        (15, 1, Some(SessionStep::Archived(5))),
        (12, 1, Some(SessionStep::Archived(6))),
        (12, -1, Some(SessionStep::Archived(4))),
        (8, 1, None),
    ] {
        assert_eq!(
            session_step(active, archived, selected, direction),
            expected,
            "selected={selected}, direction={direction}"
        );
    }
}

#[test]
fn first_archive_expansion_highlights_the_requested_row_before_runtime_confirmation() {
    let sessions = (0..7)
        .map(|index| {
            let mut session = SessionSummary::from_cached(
                index.to_string(),
                format!("/sessions/{index}").into(),
                "/project".into(),
                format!("Session {index}"),
                String::new(),
                String::new(),
                None,
                std::time::SystemTime::UNIX_EPOCH,
                0,
                crate::sessions::UsageSummary::default(),
                true,
                false,
                String::new(),
            );
            session.app_session_id = index;
            session
        })
        .collect::<Vec<_>>();
    let sessions = SessionCatalog::from(sessions);
    let archive = session_rail_lists(&sessions, &[], None, &[]).archived;
    let previous = archived_session(&archive[0]);
    let requested = archived_session(&archive[1]);

    // Expansion precedes confirmation; neither the previous selection nor an
    // older in-flight response may highlight the wrong row while loading.
    for confirmed in [&previous.path, &archived_session(&archive[0]).path] {
        let highlighted = selected_root(&sessions, Some(confirmed), Some(&requested.path));
        assert_eq!(
            highlighted.map(|session| session.app_session_id),
            Some(requested.app_session_id),
            "expanded archive highlighted a stale selection: {}",
            confirmed.display()
        );
    }
    // Once acknowledged, clearing the pending request keeps the same highlight.
    let highlighted = selected_root(&sessions, Some(&requested.path), None);
    assert_eq!(
        highlighted.map(|session| session.app_session_id),
        Some(requested.app_session_id)
    );
}

#[test]
fn navigation_handles_empty_sections_and_filtered_out_selection() {
    assert_eq!(
        session_step([], [7, 3], 7, 1),
        Some(SessionStep::Archived(1))
    );
    assert_eq!(
        session_step([7, 3], [], 3, -1),
        Some(SessionStep::Active(0))
    );
    assert_eq!(session_step([7], [3], 99, 1), None);
    assert_eq!(session_step([], [], 7, 1), None);
}
