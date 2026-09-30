use super::*;

#[test]
fn inserting_before_the_viewport_keeps_the_same_archived_row_visible() {
    let scroll = UniformListScrollHandle::new();
    let height = session_row_height(true);
    scroll
        .0
        .borrow()
        .base_handle
        .set_offset(point(px(0.), -(height + px(5.))));
    let current = RefCell::new(vec!["a".into(), "b".into(), "c".into()]);
    reconcile_archived_rows(
        &scroll,
        &current,
        vec!["new".into(), "a".into(), "b".into(), "c".into()],
    );
    assert_eq!(
        scroll.0.borrow().base_handle.offset().y,
        -(height * 2. + px(5.))
    );
}

#[test]
fn pending_archive_reveal_follows_the_session_when_rows_change() {
    let scroll = UniformListScrollHandle::new();
    scroll.scroll_to_item(1, gpui::ScrollStrategy::Nearest);
    let current = RefCell::new(vec!["a".into(), "b".into()]);
    reconcile_archived_rows(&scroll, &current, vec!["b".into(), "a".into()]);
    assert_eq!(
        scroll
            .0
            .borrow()
            .deferred_scroll_to_item
            .unwrap()
            .item_index,
        0
    );
    reconcile_archived_rows(&scroll, &current, Vec::new());
    assert!(scroll.0.borrow().deferred_scroll_to_item.is_none());
    assert_eq!(scroll.0.borrow().base_handle.offset().y, px(0.));
}
