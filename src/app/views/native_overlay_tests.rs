use super::*;
use crate::protocol::ExtensionUiRequest;
use gpui::Focusable as _;

#[cfg(target_os = "macos")]
#[gpui::test]
fn native_modal_accepts_text_and_restores_owner_focus(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::native_modal_accepts_text_and_restores_owner_focus"
        ),
        cx,
        |cx, app, runtime, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            let return_focus = cx.update(|window, cx| {
                let focus = app.read(cx).navigation.chat.focus.clone();
                focus.focus(window, cx);
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Editor;
                    app.apply_extension_request(
                        ExtensionUiRequest::Editor {
                            id: "input".into(),
                            title: "Edit message".into(),
                            prefill: Some("Before".into()),
                        },
                        app.runtime_generation,
                        cx,
                    );
                    cx.notify();
                });
                focus
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let popup =
                cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("native modal"));
            let popup_cx = gpui::VisualTestContext::from_window(popup.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.activate_window();
                window.draw(cx).clear(cx);
                assert!(
                    app.read(cx)
                        .extensions
                        .dialog_input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            });
            popup_cx.simulate_input("After");
            popup_cx.update(|window, cx| {
                assert!(
                    app.read(cx)
                        .extensions
                        .dialog_input
                        .read(cx)
                        .value()
                        .contains("After")
                );
                window.draw(cx).clear(cx);
            });
            popup_cx.simulate_keystrokes("tab shift-tab");
            popup_cx.update(|window, cx| {
                assert!(
                    gpui_base::active_focus_trap(window, cx)
                        .is_some_and(|scope| scope.contains_focused(window, cx))
                );
                app.update(cx, |app, cx| {
                    app.apply_extension_request(
                        ExtensionUiRequest::Confirm {
                            id: "queued".into(),
                            title: "Continue?".into(),
                            message: "Next request".into(),
                            timeout: None,
                        },
                        app.runtime_generation,
                        cx,
                    );
                    cx.notify();
                });
            });
            // A click outside the dialog hits its backdrop, not the editor below.
            popup_cx.simulate_click(point(px(1.0), px(1.0)), Default::default());
            cx.run_until_parked();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert_eq!(app.read(cx).overlays.modal.window, Some(popup));
                assert_eq!(
                    app.read(cx)
                        .extensions
                        .active
                        .dialog
                        .as_ref()
                        .and_then(ExtensionUiRequest::dialog_id),
                    Some("queued")
                );
                assert!(
                    app.read(cx)
                        .extensions
                        .dialog_focus
                        .contains_focused(window, cx)
                );
            });
            popup_cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let app = app.read(cx);
                assert!(app.extensions.active.dialog.is_none());
                assert!(app.overlays.modal.window.is_none());
                assert!(return_focus.is_focused(window));
                assert!(!app.workspace.native_surface_covered);
                assert!(app.workspace.native_surface_snapshot.is_none());
            });
            let responses: Vec<_> = std::iter::from_fn(|| runtime.try_recv_command()).collect();
            assert!(responses.iter().any(|command| matches!(command,
                crate::runtime::RuntimeCommand::ExtensionResponse(crate::protocol::ExtensionUiResponse::Cancelled { id, cancelled: true }) if id == "input"
            )));
        },
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn async_permission_modal_blocks_surface_switch_and_tracks_owner_size(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::async_permission_modal_blocks_surface_switch_and_tracks_owner_size"
        ),
        cx,
        |cx, app, runtime, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.workspace.surface = AppSurface::Terminal;
                    app.extensions.active.push_notification(
                        "notice".into(),
                        "Still visible".into(),
                        crate::protocol::NotifyTone::Info,
                    );
                    app.apply_extension_request(
                        ExtensionUiRequest::Confirm {
                            id: "permission".into(),
                            title: "Allow command?".into(),
                            message: "Read project files".into(),
                            timeout: None,
                        },
                        app.runtime_generation,
                        cx,
                    );
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let popup = cx.update(|_, cx| {
                app.read(cx)
                    .overlays
                    .modal
                    .window
                    .expect("async popup without a click")
            });
            cx.simulate_resize(gpui::size(px(600.0), px(450.0)));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).center_surface_switch_blocked());
                assert!(!app.read(cx).native_surface_obscured(window, cx));
            });
            let popup_cx = gpui::VisualTestContext::from_window(popup.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert_eq!(window.bounds().size, gpui::size(px(600.0), px(450.0)));
                assert!(
                    app.read(cx)
                        .extensions
                        .dialog_focus
                        .contains_focused(window, cx)
                );
            });
            assert!(popup_cx.debug_bounds("floating-notices").is_some());
            assert!(cx.debug_bounds("floating-notices").is_none());
            popup_cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            popup_cx.deactivate_window();
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert_eq!(app.read(cx).overlays.modal.window, Some(popup));
            });
            cx.simulate_visibility_change(gpui::WindowVisibility::Hidden);
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).overlays.modal.window.is_none());
            });
            cx.simulate_visibility_change(gpui::WindowVisibility::Visible);
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let reopened =
                cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("restored modal"));
            assert_ne!(popup, reopened);
            let popup_cx = gpui::VisualTestContext::from_window(reopened.into(), cx).into_mut();
            popup_cx.update(|window, cx| window.draw(cx).clear(cx));
            popup_cx.simulate_keystrokes("y");
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).extensions.active.dialog.is_none());
                assert!(app.read(cx).overlays.modal.window.is_none());
                assert!(!app.read(cx).workspace.native_surface_covered);
            });
            let responses: Vec<_> = std::iter::from_fn(|| runtime.try_recv_command()).collect();
            assert!(responses.iter().any(|command| matches!(command,
                crate::runtime::RuntimeCommand::ExtensionResponse(crate::protocol::ExtensionUiResponse::Confirmed { id, confirmed: true }) if id == "permission"
            )));
        },
    );
}
