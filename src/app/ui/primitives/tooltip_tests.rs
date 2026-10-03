use super::*;
use gpui::{AppContext as _, Styled as _, point, px, size};

#[gpui::test]
fn tooltip_delay_cancellation_grace_and_click_dismissal(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        init_tooltips(cx);
    });
    struct Trigger;
    impl Render for Trigger {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .child(
                    gpui_component::button::Button::new("trigger")
                        .label("Hover")
                        .size(px(80.))
                        .app_tooltip("Hint")
                        .debug_selector(|| "tooltip-trigger".into()),
                )
                .child(
                    gpui_component::button::Button::new("sibling")
                        .label("Sibling")
                        .size(px(80.))
                        .app_tooltip("Sibling hint")
                        .debug_selector(|| "tooltip-sibling".into()),
                )
        }
    }
    let (_, cx) = cx.add_window_view(|window, cx| {
        let trigger = cx.new(|_| Trigger);
        gpui_base::Root::new(trigger, window, cx)
    });
    let overlay = cx.update(|window, cx| tooltip_overlay(window, cx).unwrap());
    let bounds = cx.debug_bounds("tooltip-trigger").unwrap();
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.run_until_parked();
    cx.executor()
        .advance_clock(SHOW_DELAY - Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!overlay.read(cx).is_visible()));
    cx.simulate_mouse_move(point(px(300.), px(300.)), None, Default::default());
    cx.executor().advance_clock(SHOW_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!overlay.read(cx).is_visible()));
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.run_until_parked();
    cx.executor().advance_clock(SHOW_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            overlay.read(cx).content.as_ref().unwrap().0.center(),
            bounds.center()
        )
    });
    let sibling = cx.debug_bounds("tooltip-sibling").unwrap();
    for target in [sibling, bounds] {
        cx.simulate_mouse_move(target.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(GRACE_PERIOD);
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(
                overlay.read(cx).content.as_ref().unwrap().0.center(),
                target.center()
            )
        });
    }
    cx.simulate_mouse_move(point(px(300.), px(300.)), None, Default::default());
    cx.run_until_parked();
    cx.executor()
        .advance_clock(GRACE_PERIOD - Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|_, cx| assert!(overlay.read(cx).is_visible()));
    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    cx.run_until_parked();
    cx.executor().advance_clock(GRACE_PERIOD);
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert!(
            overlay.read(cx).is_visible(),
            "returning hover cancels the hide"
        )
    });
    cx.simulate_click(bounds.center(), Default::default());
    cx.update(|window, cx| {
        assert!(!overlay.read(cx).is_visible());
        overlay.update(cx, |state, cx| {
            state.last_hide = Some(Instant::now() - GRACE_PERIOD);
            state.show(
                overlay.entity_id(),
                Bounds::new(point(px(0.), px(0.)), size(px(80.), px(80.))),
                Rc::new(tooltip("Hint")),
                window,
                cx,
            );
            assert!(
                !state.is_visible(),
                "an expired grace period must use the delay"
            );
        });
    });
}
