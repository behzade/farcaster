use super::*;

#[test]
fn first_archive_expansion_does_not_scroll_past_the_selected_row() {
    let list = ListState::new(12, ListAlignment::Top, gpui::px(0.0))
        .with_uniform_item_height(THEME.layout.session_row_height);
    let rows = RefCell::new((0..12).map(|index| format!("session:{index}")).collect());
    let mut reveal = Some("session:5".to_owned());
    reveal_session_row(&list, &rows, &mut reveal);
    assert_eq!(list.logical_scroll_top().item_ix, 5);
    assert!(reveal.is_none());
    // Moving back also reveals the previous row, without a second pending request.
    reveal = Some("session:4".to_owned());
    reveal_session_row(&list, &rows, &mut reveal);
    assert_eq!(list.logical_scroll_top().item_ix, 4);
}
