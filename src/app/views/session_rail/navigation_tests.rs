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
