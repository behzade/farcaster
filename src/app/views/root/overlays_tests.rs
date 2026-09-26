#[gpui::test]
fn floating_notices_survive_sidebar_collapse_and_expire_into_history(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::floating_notices_survive_sidebar_collapse_and_expire_into_history"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.extensions.active.push_notification(
                        "test-notice".into(),
                        "A test warning".into(),
                        crate::protocol::NotifyTone::Warning,
                    );
                    cx.notify();
                });
            });
            for hidden in [false, true] {
                for collapsed in [false, true] {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            app.workspace.session_rail_hidden = hidden;
                            app.views.notification_panel.set_collapsed(collapsed);
                            cx.notify();
                        });
                        window.draw(cx).clear(cx);
                    });
                    assert!(
                        cx.debug_bounds("floating-notices").is_some(),
                        "hidden={hidden}, collapsed={collapsed}"
                    );
                }
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let state = &mut app.extensions.active;
                    let notice = state.notifications[0].clone();
                    assert!(state.remove_notification(&notice.id, notice.expires_at));
                    assert_eq!(state.notification_history.len(), 1);
                    assert_eq!(state.notification_history[0].message, "A test warning");
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("floating-notices").is_none());
        },
    );
}
