use super::*;

fn archive_rows(ids: &[&str]) -> PersistentVec<ArchivedRow> {
    ids.iter()
        .map(|id| ArchivedRow {
            identity: format!("draft:{id}"),
            item: ArchivedItem::Draft(id.to_string(), 0),
            order: (0, 0),
        })
        .collect()
}

#[test]
fn inserting_before_the_viewport_keeps_the_same_archived_row_visible() {
    let scroll = UniformListScrollHandle::new();
    let height = session_row_height(false);
    scroll
        .0
        .borrow()
        .base_handle
        .set_offset(point(px(0.), -(height + px(5.))));
    reconcile_archived_rows(
        &scroll,
        &archive_rows(&["a", "b", "c"]),
        &archive_rows(&["new", "a", "b", "c"]),
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
    let reordered = archive_rows(&["b", "a"]);
    reconcile_archived_rows(&scroll, &archive_rows(&["a", "b"]), &reordered);
    assert_eq!(
        scroll
            .0
            .borrow()
            .deferred_scroll_to_item
            .unwrap()
            .item_index,
        0
    );
    reconcile_archived_rows(&scroll, &reordered, &archive_rows(&[]));
    assert!(scroll.0.borrow().deferred_scroll_to_item.is_none());
    assert_eq!(scroll.0.borrow().base_handle.offset().y, px(0.));
}
