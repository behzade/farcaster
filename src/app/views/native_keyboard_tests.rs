use super::*;
use crate::app::{
    AppSurface, PickerScope,
    views::native_overlay::native::{NativeHost, TestHost},
};
use crate::protocol::ExtensionUiRequest;
use gpui::{AppContext as _, TestAppContext, VisualTestContext, px, size};

#[gpui::test]
fn wayland_picker_accepts_owner_keys_without_popup_activation(cx: &mut TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::wayland_picker_accepts_owner_keys_without_popup_activation"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                cx.set_global(TestHost(NativeHost::TestWayland));
                window.activate_window();
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.navigation.chat.focus.focus(window, cx);
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.simulate_keystrokes("ctrl-g space");
            cx.run_until_parked();
            let panel =
                cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("picker panel"));
            let popup_cx = VisualTestContext::from_window(panel.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(!window.is_window_active());
                assert!(window.focused_input(cx).is_some());
            });
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(window.is_window_active());
                assert!(!app.read(cx).picker_focus(cx).unwrap().is_focused(window));
            });
            cx.simulate_input("Settings");
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(app.read(cx).overlays.view.settings);
                assert_eq!(app.read(cx).workspace.surface, AppSurface::Terminal);
                assert!(!app.read(cx).workspace.native_surface_covered);
                assert!(app.read(cx).workspace.native_surface_snapshot.is_none());
            });
        },
    );
}

#[gpui::test]
fn wayland_async_request_forwards_composition_and_utf16_edits(cx: &mut TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::wayland_async_request_forwards_composition_and_utf16_edits"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                cx.set_global(TestHost(NativeHost::TestWayland));
                window.activate_window();
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.navigation.chat.focus.focus(window, cx);
                    app.apply_extension_request(
                        ExtensionUiRequest::Editor {
                            id: "async-text".into(),
                            title: "Edit".into(),
                            prefill: None,
                        },
                        app.runtime_generation,
                        cx,
                    );
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            let panel = cx.update(|_, cx| {
                app.read(cx)
                    .overlays
                    .modal
                    .window
                    .expect("async panel without a press")
            });
            let popup_cx = VisualTestContext::from_window(panel.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(!window.is_window_active());
                assert!(window.focused_input(cx).is_some());
            });
            cx.update(|window, cx| {
                let mut ime = OwnerInput {
                    popup: panel.into(),
                    offset: Point::default(),
                };
                assert!(ime.accepts_text_input(window, cx));
                ime.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                assert_eq!(ime.marked_text_range(window, cx), Some(0..1));
                ime.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                ime.replace_text_in_range(None, "日本語🙂", window, cx);
                assert!(ime.marked_text_range(window, cx).is_none());
                ime.replace_text_in_range(Some(3..5), "X", window, cx);
                let mut adjusted = None;
                assert_eq!(
                    ime.text_for_range(0..4, &mut adjusted, window, cx)
                        .as_deref(),
                    Some("日本語X")
                );
                assert!(ime.bounds_for_range(0..1, window, cx).is_some());
                assert!(app.read(cx).composer.input.read(cx).value().is_empty());
            });
            cx.simulate_keystrokes("backspace");
            cx.update(|window, cx| {
                assert!(relay_keystroke(
                    &Keystroke {
                        key: "x".into(),
                        key_char: Some("ع".into()),
                        modifiers: Modifiers::default(),
                    },
                    window,
                    cx
                ));
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx)
                        .extensions
                        .dialog_input
                        .read(cx)
                        .value()
                        .as_ref(),
                    "日本語ع"
                );
            });
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).extensions.active.dialog.is_none());
                assert!(target(window, cx).is_none());
                assert!(!cx.windows().contains(&panel.into()));
                assert_eq!(app.read(cx).workspace.surface, AppSurface::Terminal);
            });
        },
    );
}

#[gpui::test]
fn wayland_resize_closes_upper_popups_and_reanchors_modal(cx: &mut TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::wayland_resize_closes_upper_popups_and_reanchors_modal"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                cx.set_global(TestHost(NativeHost::TestWayland));
                window.activate_window();
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.navigation.chat.focus.focus(window, cx);
                    app.open_picker(PickerScope::Actions, window, cx);
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            let lower = cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("lower"));
            let upper = cx.update(|window, cx| {
                NativeHost::open_panel(
                    window,
                    window.bounds(),
                    OverlayInput::Tooltip,
                    Point::default(),
                    |_, _, cx| cx.new(|_| EmptyPanel),
                    cx,
                )
                .unwrap()
            });
            cx.simulate_resize(size(px(620.0), px(460.0)));
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
            let resized = cx.update(|_, cx| {
                assert!(!cx.windows().contains(&lower.into()));
                assert!(!cx.windows().contains(&upper.into()));
                let resized = app.read(cx).overlays.modal.window.expect("resized popup");
                assert_ne!(lower, resized);
                resized
            });
            let popup_cx = VisualTestContext::from_window(resized.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert_eq!(window.viewport_size(), size(px(620.0), px(460.0)));
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.close_picker(window, cx));
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| assert!(target(window, cx).is_none()));
        },
    );
}

#[gpui::test]
fn wayland_worker_picker_returns_to_passive_and_survives_owner_deactivation(
    cx: &mut TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::wayland_worker_picker_returns_to_passive_and_survives_owner_deactivation"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                cx.set_global(TestHost(NativeHost::TestWayland));
                window.activate_window();
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.navigation.chat.focus.focus(window, cx);
                    app.apply_extension_request(
                        ExtensionUiRequest::WorkerModel {
                            id: "worker".into(),
                            profile: "Worker".into(),
                            choices: vec![crate::protocol::WorkerModelChoice {
                                harness: crate::agents::Backend::Codex,
                                provider: "test".into(),
                                id: "test".into(),
                                name: "Test model".into(),
                                efforts: vec![],
                                service_tiers: vec![],
                            }],
                        },
                        app.runtime_generation,
                        cx,
                    );
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            let passive = cx.update(|window, cx| {
                assert!(target(window, cx).is_none());
                app.read(cx).overlays.modal.window.unwrap()
            });
            cx.update(|_, cx| {
                passive
                    .update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.set_worker_model_picker_open(true, "worker", window, cx)
                        });
                    })
                    .unwrap()
            });
            cx.run_until_parked();
            let modal = cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                let modal = app.read(cx).overlays.modal.window.unwrap();
                assert_ne!(passive, modal);
                assert!(!cx.windows().contains(&passive.into()));
                assert_eq!(
                    target(window, cx).map(|(target, _)| target),
                    Some(modal.into())
                );
                modal
            });
            let popup_cx = VisualTestContext::from_window(modal.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(
                    app.read(cx)
                        .worker_model_picker_focus(cx)
                        .unwrap()
                        .is_focused(window)
                );
                assert!(!window.is_window_active());
            });
            cx.simulate_input("Test");
            cx.deactivate_window();
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).overlays.modal.window.is_none());
                assert!(!cx.windows().contains(&modal.into()));
                assert!(
                    !window.is_window_active(),
                    "dismissal must not reactivate the app"
                );
            });
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(target(window, cx).is_some());
                let mut ime = OwnerInput {
                    popup: target(window, cx).unwrap().0,
                    offset: Point::default(),
                };
                let mut adjusted = None;
                assert_eq!(
                    ime.text_for_range(0..4, &mut adjusted, window, cx)
                        .as_deref(),
                    Some("Test")
                );
            });
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(target(window, cx).is_none());
                assert!(!app.read(cx).workspace.runtime_picker.open);
                assert!(app.read(cx).extensions.active.dialog.is_some());
                assert!(app.read(cx).overlays.modal.window.is_some());
                assert_eq!(app.read(cx).workspace.surface, AppSurface::Terminal);
            });
        },
    );
}

struct EmptyPanel;
impl gpui::Render for EmptyPanel {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        div()
    }
}

#[gpui::test]
fn wayland_nonblocking_question_forwards_keys_after_input_focus(cx: &mut TestAppContext) {
    passive_question_input(
        concat!(
            module_path!(),
            "::wayland_nonblocking_question_forwards_keys_after_input_focus"
        ),
        NativeHost::TestWayland,
        cx,
    );
}

#[gpui::test]
fn x11_nonblocking_question_forwards_keys_after_input_focus(cx: &mut TestAppContext) {
    passive_question_input(
        concat!(
            module_path!(),
            "::x11_nonblocking_question_forwards_keys_after_input_focus"
        ),
        NativeHost::TestX11,
        cx,
    );
}

fn passive_question_input(name: &'static str, host: NativeHost, cx: &mut TestAppContext) {
    use gpui::Focusable as _;
    crate::app::test_support::with_offline_app(name, cx, |cx, app, _, _| {
        cx.update(|window, cx| {
            cx.set_global(TestHost(host));
            window.activate_window();
            app.update(cx, |app, cx| {
                app.set_surface(AppSurface::Terminal, cx);
                app.navigation.chat.focus.focus(window, cx);
                app.apply_extension_request(
                    ExtensionUiRequest::Questions(crate::agents::QuestionRequest {
                        id: "async-question".into(),
                        blocking: false,
                        questions: vec![crate::agents::Question {
                            id: "choice".into(),
                            header: "Question".into(),
                            prompt: "Details?".into(),
                            options: vec![],
                            allow_free_text: true,
                            secret: false,
                        }],
                    }),
                    app.runtime_generation,
                    cx,
                );
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        cx.run_until_parked();
        let panel = cx.update(|window, cx| {
            assert!(
                target(window, cx).is_none(),
                "async questions must not take keyboard input on arrival"
            );
            app.read(cx).overlays.modal.window.expect("question panel")
        });
        let input = cx.update(|_, cx| app.read(cx).extensions.question_input.clone());
        let popup_cx = VisualTestContext::from_window(panel.into(), cx).into_mut();
        popup_cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            input.read(cx).focus_handle(cx).focus(window, cx);
            window.draw(cx).clear(cx);
            assert!(window.focused_input(cx).is_some());
            assert!(
                !window.is_window_active(),
                "ungrabbed popup retains owner keyboard focus"
            );
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(
                relay_keystroke(
                    &Keystroke {
                        key: "x".into(),
                        key_char: Some("ع".into()),
                        modifiers: Modifiers::default(),
                    },
                    window,
                    cx
                ),
                "a focused nonblocking question must receive owner keyboard input"
            );
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert_eq!(input.read(cx).value().as_ref(), "ع");
            let mut ime = OwnerInput {
                popup: panel.into(),
                offset: Point::default(),
            };
            assert!(ime.accepts_text_input(window, cx));
            ime.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            assert_eq!(ime.marked_text_range(window, cx), Some(1..2));
            ime.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(input.read(cx).value().as_ref(), "ع日本");
        });
        cx.simulate_click(gpui::point(px(400.0), px(200.0)), Modifiers::default());
        cx.run_until_parked();
        popup_cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(
                window.focused(cx).is_none(),
                "owner clicks release popup focus"
            );
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(target(window, cx).is_none());
            let mut ime = OwnerInput {
                popup: panel.into(),
                offset: Point::default(),
            };
            assert!(!ime.accepts_text_input(window, cx));
            assert!(!relay_keystroke(
                &Keystroke::parse("x").unwrap(),
                window,
                cx
            ));
            assert_eq!(input.read(cx).value().as_ref(), "ع日本");
        });
        popup_cx.update(|window, cx| {
            input.read(cx).focus_handle(cx).focus(window, cx);
            window.draw(cx).clear(cx);
        });
        cx.simulate_keystrokes("backspace");
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(input.read(cx).value().as_ref(), "ع日");
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(target(window, cx).is_none());
            assert!(!cx.windows().contains(&panel.into()));
        });
    });
}
