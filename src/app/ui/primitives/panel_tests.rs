use super::*;
use gpui::px;

fn bounds() -> ResizeBounds {
    ResizeBounds {
        height: px(160.0),
        min_height: px(76.0),
        max_height: px(320.0),
        row_height: None,
    }
}

fn row_bounds() -> ResizeBounds {
    ResizeBounds {
        row_height: Some(px(32.0)),
        ..bounds()
    }
}

#[test]
fn panel_starts_at_its_theme_height() {
    let state = ResizeState::default();

    assert_eq!(state.height(bounds()), px(160.0));
    assert!(!state.is_collapsed());
}

#[test]
fn dragging_the_separator_up_grows_the_panel_within_its_bounds() {
    let mut state = ResizeState::default();
    let bounds = bounds();

    state.begin_resize(bounds, px(400.0));
    assert!(state.update_resize(bounds, px(340.0)));
    assert_eq!(state.height(bounds), px(220.0));

    assert!(state.update_resize(bounds, px(1000.0)));
    assert_eq!(state.height(bounds), px(76.0));

    assert!(state.update_resize(bounds, px(0.0)));
    assert_eq!(state.height(bounds), px(320.0));
    assert!(!state.update_resize(bounds, px(0.0)));

    assert!(state.finish_resize());
    assert!(!state.finish_resize());
}

#[test]
fn a_released_separator_ignores_pointer_movement() {
    let mut state = ResizeState::default();
    let bounds = bounds();

    assert!(!state.update_resize(bounds, px(10.0)));
    assert_eq!(state.height(bounds), px(160.0));
}

#[test]
fn collapsing_a_panel_remembers_its_height() {
    let mut state = ResizeState::default();
    let bounds = bounds();

    state.begin_resize(bounds, px(400.0));
    state.update_resize(bounds, px(340.0));
    state.set_collapsed(true);

    assert!(state.is_collapsed());
    assert_eq!(state.height(bounds), px(220.0));

    state.set_collapsed(false);
    assert_eq!(state.height(bounds), px(220.0));
}
#[test]
fn dragging_a_rowed_surface_lands_on_whole_rows() {
    let mut state = ResizeState::default();
    let row = px(32.0);
    let bounds = row_bounds();

    state.begin_resize(bounds, px(400.0));
    state.update_resize(bounds, px(354.0));

    let height = state.height(bounds);
    assert_eq!(height, bounds.min_height + row * 4.0);
}

#[test]
fn a_rowed_surface_never_undershoots_its_first_row() {
    let mut state = ResizeState::default();
    let bounds = row_bounds();

    state.begin_resize(bounds, px(400.0));
    assert!(state.update_resize(bounds, px(1000.0)));
    assert_eq!(state.height(bounds), bounds.min_height);
}

#[test]
fn a_rowed_surface_never_leaves_a_part_row_at_the_top() {
    let mut state = ResizeState::default();
    let row = px(32.0);
    let bounds = ResizeBounds {
        height: px(76.0 + 32.0 * 5.0),
        min_height: px(76.0),
        max_height: px(76.0 + 32.0 * 7.0),
        row_height: Some(row),
    };

    state.begin_resize(bounds, px(400.0));
    assert!(state.update_resize(bounds, px(0.0)));

    let height = state.height(bounds);
    assert_eq!(height, bounds.max_height);
    assert_eq!((height - bounds.min_height) / row, 7.0);
}
