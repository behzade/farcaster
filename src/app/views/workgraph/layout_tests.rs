use super::*;

#[test]
fn split_view_requires_room_for_both_panes() {
    assert_eq!(board_layout(791.0), BoardLayoutMode::Narrow);
    assert_eq!(board_layout(792.0), BoardLayoutMode::Wide);
}
