use super::*;
use gpui::{MouseButton, point, px};

#[test]
fn collapse_requires_a_short_press_without_any_drag() {
    let started = Instant::now();
    for sidebar in [Sidebar::Sessions, Sidebar::SourceControl] {
        let mut press = SidebarResize {
            sidebar,
            origin: point(px(100.0), px(100.0)),
            started,
            moved: false,
        };
        press.track(point(px(102.0), px(102.0)));
        assert!(press.is_click(started + Duration::from_millis(249)));
        assert!(!press.is_click(started + Duration::from_millis(250)));
        press.track(point(px(100.0), px(104.0)));
        press.track(press.origin);
        assert!(!press.is_click(started + Duration::from_millis(100)));
    }
}

#[gpui::test]
fn both_dividers_collapse_on_click_but_preserve_width_after_drags_and_holds(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::both_dividers_collapse_on_click_but_preserve_width_after_drags_and_holds"
        ),
        cx,
        |cx, app, _, _| {
            for sidebar in [Sidebar::Sessions, Sidebar::SourceControl] {
                let selector = match sidebar {
                    Sidebar::Sessions => "session-rail-resize",
                    Sidebar::SourceControl => "run-panel-resize",
                };
                let width = |cx: &gpui::App| match sidebar {
                    Sidebar::Sessions => app.read(cx).views.session_rail.read(cx).width(),
                    Sidebar::SourceControl => app.read(cx).views.run_panel.read(cx).width(),
                };
                let hidden = |cx: &gpui::App| match sidebar {
                    Sidebar::Sessions => app.read(cx).workspace.session_rail_hidden,
                    Sidebar::SourceControl => app.read(cx).workspace.run_panel_hidden,
                };
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                });
                let origin = cx.debug_bounds(selector).expect("visible divider").center();
                let before = cx.update(|_, cx| width(cx));
                cx.simulate_mouse_down(origin, MouseButton::Left, Default::default());
                cx.simulate_mouse_up(origin, MouseButton::Left, Default::default());
                cx.update(|_, cx| {
                    assert!(hidden(cx), "quick click collapses the panel");
                    assert_eq!(width(cx), before);
                    let saved = crate::app::persistence::open()
                        .unwrap()
                        .load_panel_layout()
                        .unwrap()
                        .unwrap();
                    assert!(match sidebar {
                        Sidebar::Sessions => saved.session_rail_hidden,
                        Sidebar::SourceControl => saved.run_panel_hidden,
                    });
                    app.update(cx, |app, cx| match sidebar {
                        Sidebar::Sessions => app.toggle_session_rail(cx),
                        Sidebar::SourceControl => app.toggle_run_panel(cx),
                    });
                    assert_eq!(width(cx), before, "reopening preserves width");
                });
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                });
                let origin = cx
                    .debug_bounds(selector)
                    .expect("restored divider")
                    .center();
                cx.simulate_mouse_down(origin, MouseButton::Left, Default::default());
                cx.update(|_, cx| {
                    app.update(cx, |app, _| {
                        app.views.sidebar_resize.as_mut().unwrap().started -=
                            Duration::from_millis(300);
                    })
                });
                cx.simulate_mouse_up(origin, MouseButton::Left, Default::default());
                cx.update(|_, cx| {
                    assert!(!hidden(cx), "holding does not collapse");
                    assert_eq!(width(cx), before);
                });
                let destination = origin + point(px(30.0), px(0.0));
                cx.simulate_mouse_down(origin, MouseButton::Left, Default::default());
                cx.simulate_mouse_move(destination, Some(MouseButton::Left), Default::default());
                cx.simulate_mouse_up(destination, MouseButton::Left, Default::default());
                cx.update(|_, cx| {
                    assert!(!hidden(cx), "dragging does not collapse");
                    let expected = match sidebar {
                        Sidebar::Sessions => before + px(30.0),
                        Sidebar::SourceControl => before - px(30.0),
                    };
                    assert_eq!(width(cx), expected);
                });
            }
        },
    );
}
