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

#[gpui::test]
fn completion_notices_follow_focus_and_open_the_target_session(cx: &mut gpui::TestAppContext) {
    use std::sync::Arc;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::completion_notices_follow_focus_and_open_the_target_session"
        ),
        cx,
        |cx, app, runtime, project| {
            let project = project.canonicalize().unwrap();
            let project = project.as_path();
            let current = project.join("current.jsonl");
            let other = project.join("other.jsonl");
            std::fs::write(&other, "{}").unwrap();
            let session = crate::sessions::SessionSummary::from_cached(
                "other".into(),
                other.clone(),
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
            cx.update(|window, cx| {
                cx.set_app_identity("farcaster.notifications.test", "Farcaster");
                window.activate_window();
                app.update(cx, |app, _| {
                    app.project.pending_trust_command = None;
                    app.sessions.selected_draft = None;
                    app.lifecycle.pending_session_switch = None;
                    app.sessions.all = vec![session.clone()].into();
                    app.sessions.visible = vec![session].into();
                    let snapshot = Arc::make_mut(&mut app.snapshot);
                    snapshot.project = project.into();
                    snapshot.selected_session = Some(current.clone());
                    app.views.notification_panel.set_collapsed(true);
                });
            });
            let emit = |cx: &mut gpui::VisualTestContext, path: &std::path::Path| {
                runtime.send_event(
                    crate::app::runtime::RuntimeEvent::TurnCompletedNotification {
                        body: "Finished\n  the work".into(),
                        target: Some((path.into(), project.into())),
                    },
                );
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    window.draw(cx).clear(cx);
                });
            };
            emit(cx, &current);
            cx.update(|_, cx| assert!(app.read(cx).extensions.active.notifications.is_empty()));
            assert!(cx.shown_system_notifications().is_empty());

            emit(cx, &other);
            assert!(cx.shown_system_notifications().is_empty());
            cx.update(|_, cx| {
                let app = app.read(cx);
                let state = &app.extensions.active;
                assert_eq!(state.notifications.len(), 1);
                assert_eq!(
                    state.notifications[0].message,
                    "Other session\nFinished the work"
                );
            });
            let notice = cx
                .debug_bounds("session-notification")
                .expect("clickable notice");
            cx.simulate_click(notice.center(), Default::default());
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx)
                        .lifecycle
                        .pending_session_switch
                        .as_ref()
                        .map(|(path, _)| path),
                    Some(&other)
                );
            });

            cx.deactivate_window();
            emit(cx, &current);
            let native = cx.shown_system_notifications();
            assert_eq!(native.len(), 1);
            assert_eq!(native[0].body.as_ref(), "Finished\n  the work");
            cx.update(|_, cx| assert_eq!(app.read(cx).extensions.active.notifications.len(), 1));
        },
    );
}
