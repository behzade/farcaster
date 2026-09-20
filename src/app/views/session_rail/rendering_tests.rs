use super::*;
use gpui::{point, px, size};
#[test]
fn the_archive_resizes_freely_up_to_the_room_the_folders_keep() {
    let row = theme().controls.archived_preview_row;
    let region = px(600.0);
    let ceiling = region - theme().layout.folders_min;
    let mut state = crate::app::ui::primitives::ResizeState::default();
    let bounds = archived_panel_bounds(3, Some(region));
    assert_eq!(bounds.max_height, ceiling);
    assert_eq!(bounds.min_height, archived_panel_height(1));
    assert_eq!(bounds.row_height, None, "a drag is not stepped");
    assert_eq!(state.height(bounds), archived_panel_height(3));
    assert!(
        ceiling > archived_panel_height(3),
        "three chats still leave the panel room to grow"
    );
    state.begin_resize(bounds, px(1000.0));
    state.update_resize(bounds, px(1000.0) - row / 2.0);
    state.finish_resize();
    assert_eq!(
        state.height(bounds),
        archived_panel_height(3) + row / 2.0,
        "a drag lands on any pixel"
    );
    state.begin_resize(bounds, px(1000.0));
    state.update_resize(bounds, px(-1000.0));
    state.finish_resize();
    assert_eq!(
        state.height(bounds),
        ceiling,
        "a pull stops where the folder list begins"
    );
    state.begin_resize(bounds, px(0.0));
    state.update_resize(bounds, px(10_000.0));
    state.finish_resize();
    assert_eq!(
        state.height(bounds),
        archived_panel_height(1),
        "the floor is one whole chat"
    );
    assert_eq!(
        archived_panel_bounds(3, None).max_height,
        theme().layout.notice_panel_max,
        "a rail that has not been measured yet has no ceiling to apply"
    );
}

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
