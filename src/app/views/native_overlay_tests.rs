use super::*;
use crate::protocol::ExtensionUiRequest;
use gpui::Focusable as _;

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn terminal_picker_keeps_search_focus_in_its_panel(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::terminal_picker_keeps_search_focus_in_its_panel"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
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
            let panel = cx.update(|window, cx| {
                let app = app.read(cx);
                let search = app.picker_focus(cx).expect("picker search");
                assert!(
                    !search.is_focused(window),
                    "the owner must not focus an input hosted in the panel"
                );
                app.overlays.modal.window.expect("picker panel")
            });
            let panel_cx = gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
            panel_cx.update(|window, cx| {
                window.activate_window();
                window.draw(cx).clear(cx);
            });
            cx.update(|window, cx| {
                window.blur(cx);
                window.draw(cx).clear(cx);
                assert!(!app.read(cx).picker_focus(cx).unwrap().is_focused(window));
            });
            panel_cx.simulate_input("Settings");
            panel_cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(
                    app.read(cx).overlays.view.settings,
                    "typing must filter and select Settings"
                );
                assert_eq!(app.read(cx).workspace.surface, AppSurface::Terminal);
            });
        },
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn sheets_and_picker_share_the_live_modal_layer(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::sheets_and_picker_share_the_live_modal_layer"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            for surface in [AppSurface::Editor, AppSurface::Terminal] {
                for sheet in 0..8 {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            app.set_surface(surface, cx);
                            match sheet {
                                0 => app.open_settings(window, cx),
                                1 => app.open_sessions_sheet(window, cx),
                                2 => app.open_run_sheet(window, cx),
                                3 => app.open_worker_notices(window, cx),
                                4 => app.open_keybindings_help(window, cx),
                                5 => app.open_project_trust(window, cx),
                                6 => app.open_picker(crate::app::PickerScope::Actions, window, cx),
                                _ => app.open_session_import(window, cx),
                            }
                            assert!(app.modal_uses_native_host());
                            assert!(!app.workspace.native_surface_covered);
                            assert!(app.workspace.native_surface_snapshot.is_none());
                        });
                        window.draw(cx).clear(cx);
                    });
                    cx.run_until_parked();
                    cx.update(|window, cx| {
                        assert!(!app.read(cx).keyboard_overlay_focus(window, cx).unwrap().is_focused(window), "overlay focus belongs to the panel, not its owner: {surface:?} sheet {sheet}");
                    });
                    let panel =
                        cx.update(|_, cx| {
                            app.read(cx).overlays.modal.window.unwrap_or_else(|| {
                                panic!("missing panel: {surface:?} sheet {sheet}")
                            })
                        });
                    let panel_cx =
                        gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
                    panel_cx.update(|window, cx| {
                        window.activate_window();
                        window.draw(cx).clear(cx);
                        assert!(gpui_base::active_focus_trap(window, cx).is_some());
                        assert!(
                            app.read(cx)
                                .keyboard_overlay_focus(window, cx)
                                .unwrap()
                                .contains_focused(window, cx)
                        );
                        assert!(crate::app::ui::primitives::tooltip_overlay(window, cx).is_some());
                    });
                    panel_cx.run_until_parked();
                    panel_cx.simulate_keystrokes("tab shift-tab escape");
                    cx.run_until_parked();
                    cx.update(|window, cx| {
                        window.draw(cx).clear(cx);
                        assert!(
                            !app.read(cx).native_workspace_modal_active(),
                            "sheet {sheet}"
                        );
                        assert!(app.read(cx).overlays.modal.window.is_none());
                        assert_eq!(app.read(cx).workspace.surface, surface);
                    });
                }
            }
        },
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn worker_model_prompt_uses_modal_host_and_keeps_nested_picker_focus(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::worker_model_prompt_uses_modal_host_and_keeps_nested_picker_focus"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
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
            let panel = cx.update(|_, cx| {
                app.read(cx)
                    .overlays
                    .modal
                    .window
                    .expect("worker prompt panel")
            });
            cx.update(|window, cx| {
                assert!(
                    popup::uses_native_popups(window, cx),
                    "the passive request must not disable native menus and tooltips"
                );
            });
            let panel_cx = gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
            panel_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(
                    !window.is_window_active(),
                    "the collapsed model request must not take editor focus"
                );
                assert!(!app.read(cx).center_surface_switch_blocked());
                app.update(cx, |app, cx| {
                    app.set_worker_model_picker_open(true, "worker", window, cx)
                });
            });
            panel_cx.update(|window, cx| window.draw(cx).clear(cx));
            panel_cx.update(|window, cx| {
                assert!(
                    app.read(cx)
                        .worker_model_picker_focus(cx)
                        .unwrap()
                        .is_focused(window)
                );
            });
            panel_cx.simulate_input("Test");
            panel_cx.update(|window, cx| {
                let app = app.read(cx);
                assert!(app.workspace.runtime_picker.open);
                assert!(app.extensions.active.dialog.is_some());
                assert!(!app.native_surface_obscured(window, cx));
            });
            panel_cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.runtime_picker.open);
                assert!(
                    app.read(cx).extensions.active.dialog.is_some(),
                    "first Escape closes only the picker"
                );
            });
            panel_cx.update(|window, cx| window.draw(cx).clear(cx));
            panel_cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|_, cx| assert!(app.read(cx).extensions.active.dialog.is_none()));
        },
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn project_menu_hosts_submenus_and_releases_its_panel(cx: &mut gpui::TestAppContext) {
    project_menu_host_test(cx, false);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn wayland_project_menu_relays_owner_keys_and_releases_its_panel(cx: &mut gpui::TestAppContext) {
    project_menu_host_test(cx, true);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn project_menu_host_test(cx: &mut gpui::TestAppContext, wayland: bool) {
    crate::app::test_support::with_offline_app(
        if wayland {
            concat!(
                module_path!(),
                "::wayland_project_menu_relays_owner_keys_and_releases_its_panel"
            )
        } else {
            concat!(
                module_path!(),
                "::project_menu_hosts_submenus_and_releases_its_panel"
            )
        },
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                if wayland {
                    cx.set_global(native::TestHost(NativeHost::TestWayland));
                }
                window.activate_window();
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let session = crate::sessions::SessionSummary::from_cached(
                        "menu-session".into(),
                        project.join("menu.jsonl"),
                        project.into(),
                        "Menu session".into(),
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
                    app.sessions.all = vec![session.clone()].into();
                    app.sessions.visible = vec![session].into();
                    app.workspace.session_rail_hidden = false;
                    app.settings.group_sessions_by_project = true;
                    app.set_surface(AppSurface::Editor, cx);
                    app.notify_session_rail(cx);
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let trigger = cx
                .debug_bounds("session-group-menu")
                .expect("project menu trigger");
            cx.simulate_click(trigger.center(), Default::default());
            cx.run_until_parked();
            let mut panel = cx.update(|_, cx| {
                cx.windows()
                    .into_iter()
                    .find(|handle| {
                        handle
                            .update(cx, |_, window, cx| {
                                Panel::<popup::PopupWindow>::from_window(window, cx).is_some()
                            })
                            .unwrap_or(false)
                    })
                    .expect("native menu panel")
            });
            let panel_cx = gpui::VisualTestContext::from_window(panel, cx).into_mut();
            panel_cx.update(|window, cx| {
                if !wayland {
                    window.activate_window();
                }
                window.draw(cx).clear(cx);
                assert!(!app.read(cx).workspace.native_surface_covered);
                assert!(gpui_base::GlobalState::is_in_deferred_context(cx));
            });
            if wayland {
                let focus = panel_cx.update(|window, cx| window.focused(cx));
                cx.simulate_resize(gpui::size(px(1320.0), px(720.0)));
                cx.run_until_parked();
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.run_until_parked();
                panel = cx.update(|_, cx| {
                    assert!(!cx.windows().contains(&panel));
                    cx.windows()
                        .into_iter()
                        .find(|handle| {
                            handle
                                .update(cx, |_, window, cx| {
                                    Panel::<popup::PopupWindow>::from_window(window, cx).is_some()
                                })
                                .unwrap_or(false)
                        })
                        .expect("reanchored menu")
                });
                cx.update(|_, cx| {
                    panel
                        .update(cx, |_, window, cx| {
                            window.draw(cx).clear(cx);
                            assert_eq!(window.focused(cx), focus, "resize preserves menu input");
                        })
                        .unwrap()
                });
            }
            let panel_cx = gpui::VisualTestContext::from_window(panel, cx).into_mut();
            if wayland {
                cx.simulate_keystrokes("down right");
            } else {
                panel_cx.simulate_keystrokes("down right");
            }
            panel_cx.update(|window, cx| window.draw(cx).clear(cx));
            if wayland {
                cx.simulate_keystrokes("escape");
            } else {
                panel_cx.simulate_keystrokes("escape");
            }
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(!gpui_base::GlobalState::is_in_deferred_context(cx));
                assert!(panel.update(cx, |_, _, _| ()).is_err(), "menu panel closes");
                assert!(!app.read(cx).workspace.native_surface_covered);
            });
        },
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn native_tooltip_preserves_owner_focus_and_expires(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::native_tooltip_preserves_owner_focus_and_expires"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                cx.set_global(native::TestHost(NativeHost::TestWayland));
                window.replace_root(cx, |window, cx| {
                    gpui_base::Root::new(app.clone(), window, cx)
                });
                window.activate_window();
            });
            cx.run_until_parked();
            let focus = cx.update(|window, cx| {
                let focus = app.read(cx).navigation.chat.focus.clone();
                focus.focus(window, cx);
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Editor, cx);
                    cx.notify();
                });
                window.draw(cx).clear(cx);
                focus
            });
            let bar = cx.debug_bounds("workspace-bar").unwrap();
            cx.simulate_mouse_move(
                point(bar.right() - px(29.0), bar.center().y),
                None,
                Default::default(),
            );
            cx.run_until_parked();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(400));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(
                    crate::app::ui::primitives::tooltip_overlay(window, cx)
                        .unwrap()
                        .read(cx)
                        .is_visible()
                );
                assert!(focus.is_focused(window));
                assert!(!app.read(cx).workspace.native_surface_covered);
            });
            let panel = cx.update(|_, cx| {
                cx.windows()
                    .into_iter()
                    .find_map(|window| window.downcast::<popup::PopupWindow>())
                    .expect("tooltip panel without a decorated Root")
            });
            let panel_cx = gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
            panel_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(window.focused(cx).is_none());
            });
            cx.simulate_mouse_move(point(px(10.0), px(450.0)), None, Default::default());
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(600));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(focus.is_focused(window));
                assert!(panel.update(cx, |_, _, _| ()).is_err());
            });
        },
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[gpui::test]
fn picker_action_survives_its_source_popup_closing(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use std::rc::Rc;
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::picker_action_survives_its_source_popup_closing"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            let source = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Editor, cx);
                    cx.notify();
                });
                let state = cx.new(|_| popup::PopupState::default());
                let owner = app.downgrade();
                popup::prepare(
                    state.clone(),
                    true,
                    Rc::new(move |_, _| {
                        crate::app::ui::primitives::button(
                            "open-actions",
                            "Actions",
                            crate::app::ui::primitives::ButtonTone::Neutral,
                            true,
                            {
                                let owner = owner.clone();
                                move |window, cx| {
                                    let _ = owner.update(cx, |app, cx| {
                                        app.open_picker(
                                            crate::app::PickerScope::Actions,
                                            window,
                                            cx,
                                        )
                                    });
                                }
                            },
                        )
                        .debug_selector(|| "open-actions".into())
                        .into_any_element()
                    }),
                    OverlayInput::Modal,
                    None,
                    window,
                    cx,
                );
                state
            });
            cx.run_until_parked();
            let source_panel = cx.update(|_, cx| source.read(cx).window.unwrap());
            let source_cx =
                gpui::VisualTestContext::from_window(source_panel.into(), cx).into_mut();
            source_cx.update(|window, cx| {
                window.activate_window();
                window.draw(cx).clear(cx);
            });
            let button = source_cx.debug_bounds("open-actions").unwrap();
            source_cx.simulate_click(button.center(), Default::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                assert!(
                    app.read(cx).overlays.modal.window.is_some(),
                    "the active source panel must not prevent its dialog from opening"
                );
                popup::prepare(
                    source.clone(),
                    false,
                    Rc::new(|_, _| div().into_any_element()),
                    OverlayInput::Modal,
                    None,
                    window,
                    cx,
                );
            });
            cx.run_until_parked();
            let panel =
                cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("picker panel"));
            let panel_cx = gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
            panel_cx.update(|window, cx| {
                window.activate_window();
                window.draw(cx).clear(cx);
            });
            panel_cx.simulate_input("Settings");
            panel_cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(
                    app.read(cx).overlays.view.settings,
                    "picker subscription still reaches the owner after its source panel closes"
                );
                assert!(app.read(cx).navigation.picker.is_none());
                assert!(!app.read(cx).workspace.native_surface_covered);
            });
            panel_cx.simulate_keystrokes("escape");
        },
    );
}
