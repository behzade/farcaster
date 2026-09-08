use gpui::{Bounds, point, size};

use super::*;

#[test]
fn reorder_position_only_selects_the_row_under_the_pointer() {
    let row = Bounds::new(point(px(10.0), px(100.0)), size(px(200.0), px(40.0)));

    assert_eq!(
        reorder_position(&row, &point(px(20.0), px(110.0))),
        Some(ReorderPosition::Before)
    );
    assert_eq!(
        reorder_position(&row, &point(px(20.0), px(130.0))),
        Some(ReorderPosition::After)
    );
    assert_eq!(reorder_position(&row, &point(px(20.0), px(90.0))), None);
    assert_eq!(reorder_position(&row, &point(px(20.0), px(150.0))), None);
}
