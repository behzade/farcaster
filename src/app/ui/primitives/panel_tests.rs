use super::*;
use gpui::px;

fn bounds() -> PanelBounds {
    PanelBounds {
        height: px(160.0),
        min_height: px(76.0),
        max_height: px(320.0),
    }
}

#[test]
fn panel_starts_at_its_theme_height() {
    let state = PanelState::default();

    assert_eq!(state.height(bounds()), px(160.0));
    assert!(!state.is_collapsed());
}

#[test]
fn dragging_the_separator_up_grows_the_panel_within_its_bounds() {
    let mut state = PanelState::default();
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
    let mut state = PanelState::default();
    let bounds = bounds();

    assert!(!state.update_resize(bounds, px(10.0)));
    assert_eq!(state.height(bounds), px(160.0));
}

#[test]
fn collapsing_a_panel_remembers_its_height() {
    let mut state = PanelState::default();
    let bounds = bounds();

    state.begin_resize(bounds, px(400.0));
    state.update_resize(bounds, px(340.0));
    state.set_collapsed(true);

    assert!(state.is_collapsed());
    assert_eq!(state.height(bounds), px(220.0));

    state.set_collapsed(false);
    assert_eq!(state.height(bounds), px(220.0));
}
