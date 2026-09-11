use super::*;

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
    let archive = session_rail_lists(&sessions, &[], None, &[]).archived;
    let previous = &archive[INACTIVE_PREVIEW_LIMIT - 1].session;
    let requested = &archive[INACTIVE_PREVIEW_LIMIT].session;

    // Expansion precedes confirmation; neither the previous selection nor an
    // older in-flight response may highlight the wrong row while loading.
    for confirmed in [&previous.path, &archive[0].session.path] {
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
