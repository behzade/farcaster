use super::*;

#[gpui::test]
fn notification_window_follows_owner_activation_and_size(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::notification_window_follows_owner_activation_and_size"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Editor;
                    app.extensions.active.push_notification(
                        "notice".into(),
                        "A warning".into(),
                        crate::protocol::NotifyTone::Warning,
                    );
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let popup = cx.update(|_, cx| app.read(cx).overlays.notices.window.unwrap());
            cx.simulate_resize(size(px(240.0), px(200.0)));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let view = popup.read(cx).unwrap();
                assert_eq!(view.owner_size, size(px(240.0), px(200.0)));
            });
            cx.update(|_, cx| {
                gpui::AnyWindowHandle::from(popup)
                    .update(cx, |_, window, cx| {
                        window.draw(cx).clear(cx);
                        assert_eq!(window.bounds().size.width, px(240.0) - window_inset() * 2.0);
                    })
                    .unwrap();
            });
            cx.deactivate_window();
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).overlays.notices.window.is_none());
            });
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).overlays.notices.window.is_some());
            });
            let owner = cx.update(|window, _| window.window_handle());
            cx.simulate_window_visibility_change(owner, gpui::WindowVisibility::Hidden);
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).overlays.notices.window.is_none());
            });
            cx.simulate_window_visibility_change(owner, gpui::WindowVisibility::Visible);
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).overlays.notices.window.is_some());
                app.update(cx, |app, cx| {
                    app.overlays.view.sessions = true;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            cx.update(|_, cx| assert!(app.read(cx).overlays.notices.window.is_none()));
            if cfg!(any(target_os = "macos", target_os = "linux")) {
                let panel =
                    cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("sheet panel"));
                let panel_cx = gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
                panel_cx.update(|window, cx| window.draw(cx).clear(cx));
                assert!(panel_cx.debug_bounds("floating-notices").is_some());
                assert!(cx.debug_bounds("floating-notices").is_none());
            } else {
                assert!(cx.debug_bounds("floating-notices").is_some());
            }
        },
    );
}

#[gpui::test]
fn notification_window_preserves_owner_focus_and_expires_after_the_last_notice(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::notification_window_preserves_owner_focus_and_expires_after_the_last_notice"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            for surface in [AppSurface::Editor, AppSurface::Terminal] {
                let focus = cx.update(|window, cx| {
                    let focus = cx.focus_handle();
                    focus.focus(window, cx);
                    app.update(cx, |app, cx| {
                        app.workspace.surface = surface;
                        for id in ["first", "last"] {
                            app.extensions.active.push_notification(
                                id.into(),
                                "A warning".into(),
                                crate::protocol::NotifyTone::Warning,
                            );
                        }
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                    focus
                });
                let popup = cx.update(|window, cx| {
                    let app = app.read(cx);
                    assert_eq!(window.focused(cx), Some(focus.clone()));
                    assert!(window.is_window_active());
                    assert!(!app.workspace.native_surface_covered);
                    assert!(!app.native_surface_obscured(window, cx));
                    assert!(!app.center_surface_switch_blocked());
                    app.overlays.notices.window.expect("native notice window")
                });
                assert!(cx.debug_bounds("floating-notices").is_none());
                cx.update(|_, cx| {
                    gpui::AnyWindowHandle::from(popup)
                        .update(cx, |_, window, cx| {
                            window.draw(cx).clear(cx);
                            assert!(!window.is_window_active());
                        })
                        .unwrap();
                });
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        let first = app.extensions.active.notifications[0].clone();
                        app.extensions
                            .active
                            .remove_notification(&first.id, first.expires_at);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                cx.update(|_, cx| assert_eq!(app.read(cx).overlays.notices.window, Some(popup)));
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        let last = app.extensions.active.notifications[0].clone();
                        app.extensions
                            .active
                            .remove_notification(&last.id, last.expires_at);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                cx.update(|window, cx| {
                    assert!(app.read(cx).overlays.notices.window.is_none());
                    assert_eq!(window.focused(cx), Some(focus.clone()));
                    assert!(
                        !app.read(cx)
                            .extensions
                            .active
                            .notification_history
                            .is_empty()
                    );
                });
            }
        },
    );
}

#[gpui::test]
fn notification_actions_run_in_the_owner_window(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::notification_actions_run_in_the_owner_window"
        ),
        cx,
        |cx, app, _, project| {
            let project = project.canonicalize().unwrap();
            let project = project.as_path();
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .expect("trust the test project");
            let path = project.join("other.jsonl");
            std::fs::write(&path, "{}").unwrap();
            let session = crate::sessions::SessionSummary::from_cached(
                "other".into(),
                path.clone(),
                project.into(),
                "Other session".into(),
                String::new(),
                String::new(),
                None,
                std::time::SystemTime::now(),
                0,
                Default::default(),
                false,
                false,
                String::new(),
            );
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.project.pending_trust_command = None;
                    app.sessions.selected_draft = None;
                    app.lifecycle.pending_session_switch = None;
                    app.sessions.all = vec![session.clone()].into();
                    app.sessions.visible = vec![session].into();
                    app.workspace.surface = AppSurface::Editor;
                    app.activity
                        .system_notification_targets
                        .insert("other".into(), (path.clone(), project.into()));
                    app.extensions.active.push_notification(
                        "other".into(),
                        "Other session\nFinished".into(),
                        crate::protocol::NotifyTone::Info,
                    );
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let popup = cx.update(|_, cx| app.read(cx).overlays.notices.window.unwrap());
            let mut popup_cx = gpui::VisualTestContext::from_window(popup.into(), cx);
            popup_cx.run_until_parked();
            popup_cx.update(|window, cx| window.draw(cx).clear(cx));
            let bounds = popup_cx
                .debug_bounds("session-notification")
                .expect("clickable notification");
            popup_cx.update(|window, _| {
                assert!(
                    Bounds::new(point(px(0.0), px(0.0)), window.viewport_size())
                        .contains(&bounds.center()),
                    "notice={bounds:?}, viewport={:?}",
                    window.viewport_size()
                );
            });
            popup_cx.simulate_click(bounds.center(), Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|window, cx| {
                let app = app.read(cx);
                let selected = app
                    .lifecycle
                    .pending_session_switch
                    .as_ref()
                    .map(|(path, _)| path)
                    .or(app.snapshot.selected_session.as_ref());
                assert_eq!(selected, Some(&path));
                assert_eq!(app.workspace.surface, AppSurface::Chat);
                assert!(app.chat_composer_focus(cx).is_focused(window));
            });
        },
    );
}
