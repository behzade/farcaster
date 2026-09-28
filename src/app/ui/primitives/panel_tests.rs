use super::*;
use gpui::px;

fn bounds() -> ResizeBounds {
    ResizeBounds {
        height: px(160.0),
        min_height: px(76.0),
        max_height: px(320.0),
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

struct ScrollPanels;

impl gpui::Render for ScrollPanels {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .w(px(240.0))
            .flex()
            .flex_col()
            .children(["archive", "notifications"].map(|id| {
                Panel::new(id, &ResizeState::default(), bounds(), id).children([div()
                    .h(px(320.0))
                    .flex_none()
                    .debug_selector(move || id.to_string())
                    .into_any_element()])
            }))
    }
}

#[gpui::test]
fn panel_scrolls_within_its_bounds_independently(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|_, _| ScrollPanels);
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let archive = cx.debug_bounds("archive").unwrap();
    let notice = cx.debug_bounds("notifications").unwrap();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: notice.origin + gpui::point(px(10.0), px(10.0)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-100.0))),
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(cx.debug_bounds("notifications").unwrap().top() < notice.top());
    assert_eq!(cx.debug_bounds("archive").unwrap(), archive);
}
