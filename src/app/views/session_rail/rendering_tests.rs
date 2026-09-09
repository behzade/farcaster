use super::*;
use gpui::{point, size};

#[test]
fn empty_space_below_last_row_targets_the_end() {
    let viewport = Bounds::new(point(px(10.0), px(20.0)), size(px(200.0), px(400.0)));
    let last_row = Bounds::new(point(px(10.0), px(120.0)), size(px(200.0), px(40.0)));
    for y in [160.0, 200.0, 419.0] {
        assert_eq!(
            session_list_end_target(viewport, Some((7, last_row)), point(px(50.0), px(y))),
            Some(7)
        );
    }
    for pointer in [
        point(px(50.0), px(140.0)),
        point(px(50.0), px(421.0)),
        point(px(211.0), px(200.0)),
    ] {
        assert_eq!(
            session_list_end_target(viewport, Some((7, last_row)), pointer),
            None
        );
    }
    assert_eq!(
        session_list_end_target(viewport, None, point(px(50.0), px(200.0))),
        None
    );

    let offscreen_row = Bounds::new(point(px(10.0), px(500.0)), last_row.size);
    assert_eq!(
        session_list_end_target(
            viewport,
            Some((7, offscreen_row)),
            point(px(50.0), px(200.0))
        ),
        None
    );
}
