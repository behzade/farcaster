use super::*;
use crate::app::workspace::worker_tasks::WorkerProfileEdit;
use gpui::VisualTestContext;

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).expect(selector);
    cx.simulate_click(bounds.center(), Default::default());
    draw(cx);
}

fn press_enter(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("enter");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    draw(cx);
}

#[gpui::test]
fn voice_setting_requires_hex_and_remembers_the_toggle(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::voice_setting_requires_hex_and_remembers_the_toggle"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
            cx.run_until_parked();
            draw(cx);
            let content = cx.debug_bounds("settings-content").unwrap();
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: content.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-100_000.0))),
                ..Default::default()
            });
            draw(cx);
            assert!(cx.debug_bounds("settings-voice").is_some());
            click(cx, "voice-toggle");
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.voice.available);
                assert!(!app.read(cx).voice_enabled());
                assert!(
                    app.read(cx).settings.voice_enabled,
                    "disabled button must not change preference"
                );
                app.update(cx, |app, cx| {
                    app.workspace.voice.available = true;
                    cx.notify();
                });
            });
            draw(cx);
            cx.update(|_, cx| {
                assert!(
                    app.read(cx).voice_enabled(),
                    "default on once Hex is available"
                )
            });
            click(cx, "voice-toggle");
            cx.update(|_, cx| assert!(!app.read(cx).settings.voice_enabled));
            assert!(
                !crate::app::persistence::open()
                    .unwrap()
                    .load_voice_enabled()
                    .unwrap()
            );
            click(cx, "voice-toggle");
            cx.update(|_, cx| assert!(app.read(cx).voice_enabled()));
            assert!(
                crate::app::persistence::open()
                    .unwrap()
                    .load_voice_enabled()
                    .unwrap()
            );
        },
    );
}

#[gpui::test]
fn tabs_keep_worker_drafts_and_move_focus_to_visible_controls(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::tabs_keep_worker_drafts_and_move_focus_to_visible_controls"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
            draw(cx);
            let content = cx.debug_bounds("settings-content").unwrap();
            let font = cx.debug_bounds("settings-font-size").unwrap();
            assert!(font.bottom() <= content.bottom());
            assert!(cx.debug_bounds("settings-workers").is_none());
            assert!(content.bottom() <= cx.debug_bounds("close-settings").unwrap().top());

            click(cx, "settings-tab-General");
            cx.update(|window, cx| {
                assert!(
                    app.read(cx).settings.tab_focus[0].is_focused(window),
                    "General focused after click"
                )
            });
            cx.simulate_keystrokes("tab");
            draw(cx);
            cx.update(|window, cx| {
                assert!(
                    app.read(cx).settings.tab_focus[1].is_focused(window),
                    "Workers focused after Tab"
                )
            });
            press_enter(cx);
            assert!(cx.debug_bounds("settings-workers").is_some());
            assert!(cx.debug_bounds("settings-general").is_none());
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.edit_worker_profile(None, window, cx);
                    let Some(WorkerProfileEdit::Name { input, .. }) =
                        &app.workspace.worker_profile_editor.edit
                    else {
                        panic!("profile draft");
                    };
                    input.update(cx, |input, cx| input.set_value("unfinished", window, cx));
                    app.workspace.worker_profile_editor.error =
                        Some("Finish the profile description".into());
                })
            });
            draw(cx);
            click(cx, "settings-tab-Connections");
            cx.update(|window, cx| {
                assert!(
                    app.read(cx).settings.tab_focus[SettingsTab::Connections as usize]
                        .is_focused(window)
                );
            });
            assert!(cx.debug_bounds("harness-profile-form").is_none());
            click(cx, "toggle-harness-profile-form");
            assert!(cx.debug_bounds("harness-profile-form").is_some());
            cx.update(|window, cx| {
                let input = app.read(cx).settings.harness_profile_name.clone();
                input.update(cx, |input, cx| {
                    input.set_value("unfinished harness", window, cx);
                    input.focus(window, cx);
                });
            });
            click(cx, "toggle-harness-profile-form");
            assert!(cx.debug_bounds("harness-profile-form").is_none());
            cx.update(|window, cx| {
                assert!(app.read(cx).settings.harness_form_focus.is_focused(window));
            });
            press_enter(cx);
            assert!(cx.debug_bounds("harness-profile-form").is_some());
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx)
                        .settings
                        .harness_profile_name
                        .read(cx)
                        .value()
                        .as_str(),
                    "unfinished harness"
                );
            });
            click(cx, "settings-tab-Workers");
            cx.update(|_, cx| {
                let editor = &app.read(cx).workspace.worker_profile_editor;
                let Some(WorkerProfileEdit::Name { input, .. }) = &editor.edit else {
                    panic!("draft retained")
                };
                assert_eq!(input.read(cx).value().as_str(), "unfinished");
                assert_eq!(
                    editor.error.as_deref(),
                    Some("Finish the profile description")
                );
            });
            cx.simulate_keystrokes("escape");
            draw(cx);
            cx.update(|_, cx| assert!(!app.read(cx).overlays.view.settings));
        },
    );
}

#[gpui::test]
fn theme_editor_replaces_browsing_and_returns_focus_without_losing_invalid_input(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::theme_editor_replaces_browsing_and_returns_focus_without_losing_invalid_input"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_settings(window, cx);
                    app.settings.tab = SettingsTab::Appearance;
                    app.create_theme_from("Farcaster", window, cx);
                })
            });
            draw(cx);
            assert!(cx.debug_bounds("settings-theme-editor").is_some());
            assert!(cx.debug_bounds("settings-theme-list").is_none());
            let input = cx.update(|_, cx| app.read(cx).settings.themes.tokens[0].1.clone());
            cx.update(|window, cx| {
                input.update(cx, |input, cx| {
                    input.set_value("invalid hex", window, cx);
                    cx.emit(gpui_component::input::InputEvent::Change);
                })
            });
            draw(cx);
            click(cx, "settings-tab-General");
            click(cx, "settings-tab-Appearance");
            cx.update(|_, cx| {
                assert_eq!(input.read(cx).value().as_str(), "invalid hex");
            });
            assert!(cx.debug_bounds("settings-theme-list").is_none());
            click(cx, "theme-editor-toggle");
            assert!(cx.debug_bounds("settings-theme-list").is_some());
            assert!(cx.debug_bounds("settings-theme-editor").is_none());
            cx.update(|window, cx| {
                assert!(app.read(cx).settings.theme_editor_focus.is_focused(window))
            });
            press_enter(cx);
            assert!(cx.debug_bounds("settings-theme-editor").is_some());
            cx.update(|_, cx| assert_eq!(input.read(cx).value().as_str(), "invalid hex"));
        },
    );
}

#[gpui::test]
fn theme_rows_activate_from_empty_space_and_keyboard(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::theme_rows_activate_from_empty_space_and_keyboard"
        ),
        cx,
        |cx, app, _, _| {
            let expected = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_settings(window, cx);
                    app.settings.tab = SettingsTab::Appearance;
                    let themes = app.settings.themes.library.display_order();
                    [themes[1].name.clone(), themes[2].name.clone()]
                })
            });
            draw(cx);
            let row = cx.debug_bounds("theme-select-1").unwrap();
            cx.simulate_click(
                gpui::point(row.right() - gpui::px(10.0), row.center().y),
                Default::default(),
            );
            draw(cx);
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx).settings.themes.library.selected_name(),
                    expected[0]
                )
            });
            cx.simulate_keystrokes("tab");
            press_enter(cx);
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx).settings.themes.library.selected_name(),
                    expected[1]
                )
            });
        },
    );
}

#[gpui::test]
fn worker_model_fields_use_rows_and_fit_the_settings_viewport(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::worker_model_fields_use_rows_and_fit_the_settings_viewport"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_settings(window, cx);
                    app.settings.tab = SettingsTab::Workers;
                    app.workspace.worker_profile_editor.profiles[0].models.push(
                        crate::agents::WorkerExecution {
                            harness: crate::agents::Backend::Pi,
                            provider: "provider-with-a-long-name".into(),
                            model: "Long model name with context length and version information"
                                .into(),
                            effort: Some("medium".into()),
                            service_tier: Some("standard".into()),
                        },
                    );
                })
            });
            for (width, height) in [(1240.0, 820.0), (960.0, 700.0)] {
                cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                draw(cx);
                let [harness, provider, model, effort, tier] = [
                    "worker-harness",
                    "worker-provider",
                    "worker-model",
                    "worker-effort",
                    "worker-service-tier",
                ]
                .map(|selector| cx.debug_bounds(selector).expect(selector));
                assert_eq!(harness.top(), provider.top());
                assert!(harness.right() <= provider.left());
                assert!(harness.bottom() <= model.top());
                assert!(model.bottom() <= effort.top());
                assert_eq!(effort.top(), tier.top());
                assert!(effort.right() <= tier.left());
                assert_eq!(model.left(), harness.left());
                assert_eq!(model.right(), provider.right());
                let card = cx.debug_bounds("worker-model-card").unwrap();
                let content = cx.debug_bounds("settings-content").unwrap();
                assert!(card.right() <= content.right());
                assert!(card.bottom() <= content.bottom());
            }
        },
    );
}

#[gpui::test]
fn voice_shortcut_records_cancels_rejects_conflicts_and_resets(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::voice_shortcut_records_cancels_rejects_conflicts_and_resets"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
            cx.run_until_parked();
            draw(cx);
            let content = cx.debug_bounds("settings-content").unwrap();
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: content.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-100_000.0))),
                ..Default::default()
            });
            draw(cx);
            let press = |cx: &mut VisualTestContext, key: &str| {
                cx.simulate_keystrokes(key);
                cx.simulate_event(gpui::KeyUpEvent {
                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                });
                draw(cx);
            };
            click(cx, "voice-shortcut");
            press(cx, "ctrl-g");
            cx.update(|_, cx| {
                assert!(app.read(cx).workspace.voice.capturing_shortcut);
                assert!(app.read(cx).workspace.voice.settings_error.is_some());
                assert!(app.read(cx).settings.voice_shortcut.is_none());
                assert!(app.read(cx).navigation.chat.activation.hint().is_none());
            });
            press(cx, &crate::app::ui::keybindings::application_key("n"));
            cx.update(|_, cx| {
                assert!(app.read(cx).workspace.voice.capturing_shortcut);
                assert!(
                    app.read(cx)
                        .workspace
                        .voice
                        .settings_error
                        .as_ref()
                        .unwrap()
                        .contains("Already used")
                );
            });
            press(cx, "escape");
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.voice.capturing_shortcut);
                assert!(app.read(cx).overlays.view.settings);
            });
            click(cx, "voice-shortcut");
            press(cx, "ctrl-alt-v");
            cx.update(|_, cx| {
                assert!(!app.read(cx).workspace.voice.capturing_shortcut);
                assert_eq!(
                    app.read(cx).settings.voice_shortcut.as_deref(),
                    Some("ctrl-alt-v")
                );
            });
            assert_eq!(
                crate::app::persistence::open()
                    .unwrap()
                    .load_voice_shortcut()
                    .unwrap()
                    .as_deref(),
                Some("ctrl-alt-v")
            );
            click(cx, "voice-shortcut-reset");
            cx.update(|_, cx| assert!(app.read(cx).settings.voice_shortcut.is_none()));
            assert!(
                crate::app::persistence::open()
                    .unwrap()
                    .load_voice_shortcut()
                    .unwrap()
                    .is_none()
            );
        },
    );
}
