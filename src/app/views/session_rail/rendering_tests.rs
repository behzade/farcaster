use super::*;
use crate::app::ui::primitives::panel_bounds;
use gpui::{point, px, size};
#[test]
fn the_archive_resizes_freely_up_to_the_room_the_stack_measured() {
    let row = theme().controls.archived_preview_row;
    let room = px(472.0);
    let mut state = crate::app::ui::primitives::ResizeState::default();
    let bounds = panel_bounds(room, rail_panel_slot(RailPanel::Archived, 3, false));
    assert_eq!(bounds.max_height, room);
    assert_eq!(bounds.min_height, archived_panel_height(1));
    assert_eq!(state.height(bounds), archived_panel_height(3));
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
        room,
        "a pull stops at the room the chat list left"
    );
    state.begin_resize(bounds, px(0.0));
    state.update_resize(bounds, px(10_000.0));
    state.finish_resize();
    assert_eq!(
        state.height(bounds),
        archived_panel_height(1),
        "the floor is one whole chat"
    );
}

#[test]
fn a_minimized_panel_is_only_its_header() {
    let header = theme().controls.icon_button;
    let archived = rail_panel_slot(RailPanel::Archived, 3, true);
    let notifications = rail_panel_slot(RailPanel::Notifications, 0, true);
    assert_eq!(archived.floor, header);
    assert_eq!(archived.preferred, header);
    assert_eq!(panel_bounds(px(472.0), archived).height, header);
    assert_eq!(notifications.floor, header);
    assert_eq!(panel_bounds(px(300.0), notifications).height, header);
    assert!(
        rail_panel_slot(RailPanel::Notifications, 0, false).floor > header,
        "an open panel asks for more than its header"
    );
    assert!(
        theme().layout.folders_min >= theme().layout.session_row_height * 4.0,
        "the chats keep four rows whatever grows under them"
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
