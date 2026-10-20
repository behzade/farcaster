use super::{
    choice_copy, composer_primary_action, dialog_copy, dialog_number_selection,
    numbered_dialog_choice, plain_text_html,
};
use crate::protocol::ExtensionUiRequest;
use gpui::Focusable as _;

#[gpui::test]
fn concurrent_question_keeps_composer_focus_and_sends_typed_answer(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::concurrent_question_keeps_composer_focus_and_sends_typed_answer"
        ),
        cx,
        |cx, app, runtime, _| {
            cx.update(|window, cx| {
                window.activate_window();
                app.update(cx, |app, cx| {
                    app.set_surface(crate::app::AppSurface::Chat, cx);
                    app.composer.focus.focus(window, cx);
                    app.apply_extension_request(
                        ExtensionUiRequest::Questions(crate::agents::QuestionRequest {
                            id: "question".into(),
                            blocking: false,
                            questions: vec![crate::agents::Question {
                                id: "answer".into(),
                                header: "Question".into(),
                                prompt: "Details?".into(),
                                options: vec![],
                                allow_free_text: true,
                                secret: true,
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
            let field = cx.debug_bounds("question-answer-field").unwrap();
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(app.read(cx).composer.focus.is_focused(window));
                assert!(!app.read(cx).center_surface_switch_blocked());
                let input = app.read(cx).extensions.question_input.read(cx);
                let bounds = input.input_bounds();
                let theme = crate::app::ui::theme::theme();
                let inset = theme.space.sm + theme.border;
                assert!(bounds.top() >= field.top() + inset);
                assert!(bounds.bottom() <= field.bottom() - inset);
                assert!(
                    app.read(cx)
                        .extensions
                        .question_input
                        .read(cx)
                        .context_menu_capabilities()
                        .is_masked()
                );
            });
            cx.simulate_input("Keep working");
            cx.update(|window, cx| {
                assert!(
                    app.read(cx)
                        .composer
                        .input
                        .read(cx)
                        .value()
                        .contains("Keep working")
                );
                app.update(cx, |app, cx| {
                    app.extensions
                        .question_input
                        .update(cx, |input, cx| input.set_value("Secret answer", window, cx));
                    app.apply_extension_request(
                        ExtensionUiRequest::Confirm {
                            id: "approval".into(),
                            title: "Approval".into(),
                            message: "Continue?".into(),
                            timeout: None,
                        },
                        app.runtime_generation,
                        cx,
                    );
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(
                        app.extensions.active.dialog.as_ref().unwrap().dialog_id(),
                        Some("approval")
                    );
                    app.cancel_dialog(window, cx);
                });
                window.draw(cx).clear(cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(
                        app.extensions.question_input.read(cx).value().as_ref(),
                        "Secret answer"
                    );
                    app.extensions
                        .question_input
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                });
                window.draw(cx).clear(cx);
            });
            cx.simulate_keystrokes("shift-enter");
            cx.update(|_, cx| {
                assert!(app.read(cx).extensions.active.dialog.is_some());
            });
            cx.simulate_keystrokes("enter");
            cx.update(|_, cx| {
                assert!(app.read(cx).extensions.active.dialog.is_none());
                assert!(
                    app.read(cx)
                        .extensions
                        .question_input
                        .read(cx)
                        .value()
                        .is_empty()
                );
            });
            let responses: Vec<_> = std::iter::from_fn(|| runtime.try_recv_command()).collect();
            assert!(responses.iter().any(|command| matches!(command,
                crate::runtime::RuntimeCommand::ExtensionResponse(crate::protocol::ExtensionUiResponse::Questions(response))
                if response.id=="question" && response.answers["answer"]==["Secret answer"] && !response.cancelled
            )));
        },
    );
}

#[gpui::test]
fn saved_prompt_body_keeps_actions_in_view_for_large_text(cx: &mut gpui::TestAppContext) {
    use gpui::{
        AppContext as _, InteractiveElement as _, IntoElement as _, ParentElement as _,
        ScrollDelta, ScrollWheelEvent, Styled as _, div, point, px, size,
    };

    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    struct SavedPromptView {
        id: i64,
        text: String,
    }
    impl gpui::Render for SavedPromptView {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .w(px(360.0))
                .debug_selector(|| "saved-prompt-row".into())
                .child(
                    div()
                        .debug_selector(|| "saved-prompt-warning".into())
                        .child("Delivery unconfirmed"),
                )
                .child(
                    super::queue::message_body(
                        format!("saved-prompt-body-{}", self.id),
                        &self.text,
                    )
                    .debug_selector(|| "saved-prompt-body".into())
                    .child(
                        div()
                            .debug_selector(|| "saved-prompt-tail".into())
                            .child("end of saved text"),
                    ),
                )
                .child(
                    div()
                        .debug_selector(|| "saved-prompt-actions".into())
                        .child("Send again   Remove"),
                )
        }
    }
    let draw = |cx: &mut gpui::VisualTestContext, view: &gpui::Entity<SavedPromptView>| {
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(360.0), px(500.0)),
            |_, _| view.clone().into_any_element(),
        );
    };
    for (index, text) in ["line\n".repeat(500), "x".repeat(2_000)]
        .into_iter()
        .enumerate()
    {
        let id = 143 + index as i64;
        let view = cx.new(|_| SavedPromptView { id, text });
        draw(cx, &view);
        let row = cx.debug_bounds("saved-prompt-row").expect("row rendered");
        let warning = cx
            .debug_bounds("saved-prompt-warning")
            .expect("warning rendered");
        let body = cx.debug_bounds("saved-prompt-body").expect("body rendered");
        let actions = cx
            .debug_bounds("saved-prompt-actions")
            .expect("actions rendered");
        assert!(row.right() <= px(360.0));
        assert!(body.size.height > px(0.0));
        assert!(body.size.height <= crate::app::ui::theme::theme().layout.tool_max_height);
        assert!(body.right() <= row.right());
        assert!(warning.top() >= px(0.0));
        assert!(warning.bottom() <= body.top());
        assert!(actions.top() >= body.bottom());
        assert!(actions.bottom() <= px(500.0));

        if index == 0 {
            assert!(
                cx.debug_bounds("saved-prompt-tail")
                    .is_none_or(|tail| tail.top() > body.bottom())
            );
            cx.simulate_event(ScrollWheelEvent {
                position: body.center(),
                delta: ScrollDelta::Pixels(point(px(0.0), px(-100_000.0))),
                ..Default::default()
            });
            draw(cx, &view);
            let tail = cx
                .debug_bounds("saved-prompt-tail")
                .expect("saved text tail rendered after scrolling");
            assert!(tail.top() >= body.top() - px(1.0));
            assert!(tail.bottom() <= body.bottom() + px(1.0));
        }
    }
}

#[test]
fn primary_action_only_appears_for_submit_ready_content() {
    assert_eq!(composer_primary_action(false, true, false, false), None);
    assert_eq!(composer_primary_action(false, true, false, true), None);
    assert_eq!(composer_primary_action(true, false, false, false), None);
    assert_eq!(
        composer_primary_action(true, true, false, false),
        Some("Send")
    );
    assert_eq!(
        composer_primary_action(true, true, false, true),
        Some("Steer")
    );
    assert_eq!(
        composer_primary_action(true, true, true, false),
        Some("Run")
    );
}

#[test]
fn dialog_copy_preserves_extension_owned_copy() {
    let (heading, prompt) = dialog_copy("File access request\nAllow bash to write to /work/file?");
    assert_eq!(heading.as_ref(), "File access request");
    assert_eq!(
        prompt.as_ref().map(AsRef::as_ref),
        Some("Allow bash to write to /work/file?")
    );
}

#[test]
fn dialog_text_does_not_interpret_tilde_paths_as_markdown() {
    let text = "write file  \"~/Projects/one\"\nwrite file  \"~/Projects/two\"";

    assert_eq!(
        plain_text_html(text).as_ref(),
        "write file  &quot;~/Projects/one&quot;<br>write file  &quot;~/Projects/two&quot;"
    );
}

#[test]
fn choice_copy_preserves_extension_owned_copy() {
    let (label, detail) = choice_copy("Add to project policy");
    assert_eq!(label.as_ref(), "Add to project policy");
    assert_eq!(detail, None);
}

#[test]
fn number_keys_match_the_displayed_shortcuts() {
    let request = ExtensionUiRequest::Select {
        id: "question-1".into(),
        title: "Choose".into(),
        options: (1..=6).map(|number| format!("Option {number}")).collect(),
        timeout: None,
    };

    for number in 1..=5 {
        let key = number.to_string();
        let option = format!("Option {number}");
        assert_eq!(
            dialog_number_selection(&request, &key),
            Some(("question-1", option.as_str()))
        );
        assert_eq!(
            numbered_dialog_choice(number - 1, &option),
            format!("[{key}] {option}")
        );
    }
    for key in ["0", "6", "enter", "space", " "] {
        assert_eq!(dialog_number_selection(&request, key), None);
    }
}

#[test]
fn number_keys_ignore_missing_options_and_non_select_dialogs() {
    let select = ExtensionUiRequest::Select {
        id: "question-1".into(),
        title: "Choose".into(),
        options: vec!["Only".into()],
        timeout: None,
    };
    let input = ExtensionUiRequest::Input {
        id: "question-2".into(),
        title: "Explain".into(),
        placeholder: None,
        timeout: None,
    };

    assert_eq!(dialog_number_selection(&select, "2"), None);
    assert_eq!(dialog_number_selection(&input, "1"), None);
}

#[gpui::test]
fn question_progress_survives_session_switches(cx: &mut gpui::TestAppContext) {
    use crate::app::*;
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::question_progress_survives_session_switches"
        ),
        cx,
        |cx, app, runtime, project| {
            let request = ExtensionUiRequest::Questions(crate::agents::QuestionRequest {
                id: "batch".into(),
                blocking: false,
                questions: ["first", "second"]
                    .into_iter()
                    .map(|id| crate::agents::Question {
                        id: id.into(),
                        header: "Question".into(),
                        prompt: id.into(),
                        options: vec![],
                        allow_free_text: true,
                        secret: false,
                    })
                    .collect(),
            });
            for (generation, session) in [(1, "one"), (2, "two"), (3, "one")] {
                runtime.send_event(RuntimeEvent::SessionReset {
                    generation,
                    preserve_submission: false,
                });
                runtime.send_event(RuntimeEvent::Snapshot {
                    generation,
                    snapshot: Arc::new(RuntimeSnapshot {
                        selected_session: Some(project.join(session)),
                        project: project.into(),
                        ..Default::default()
                    }),
                });
                runtime.send_event(RuntimeEvent::ExtensionUi {
                    generation,
                    request: request.clone(),
                    system_notification_target: None,
                });
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.drain_runtime(cx);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                cx.run_until_parked();
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        if generation == 1 {
                            assert!(
                                app.extensions
                                    .active
                                    .answer_question("batch", "first", "Saved answer".into())
                                    .unwrap()
                                    .is_none()
                            );
                            app.extensions.pending_dialog_setup = true;
                        } else if generation == 2 {
                            assert_eq!(
                                app.extensions.active.current_question().unwrap().id,
                                "first",
                                "sessions must not share progress"
                            );
                        } else {
                            assert_eq!(
                                app.extensions.active.current_question().unwrap().id,
                                "second"
                            );
                            assert_eq!(
                                app.extensions.question_input.read(cx).value().as_ref(),
                                "Unfinished answer"
                            );
                            let Some(crate::protocol::ExtensionUiResponse::Questions(response)) =
                                app.extensions
                                    .active
                                    .answer_question("batch", "second", "Finished answer".into())
                                    .unwrap()
                            else {
                                panic!("response");
                            };
                            assert_eq!(response.answers["first"], vec!["Saved answer"]);
                        }
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                cx.run_until_parked();
                if generation == 1 {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            app.extensions.question_input.update(cx, |input, cx| {
                                input.set_value("Unfinished answer", window, cx)
                            });
                        })
                    });
                }
            }
        },
    );
}

#[gpui::test]
fn blocking_option_question_has_visible_native_focus(cx: &mut gpui::TestAppContext) {
    use crate::app::*;
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::blocking_option_question_has_visible_native_focus"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                window.activate_window();
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.navigation.chat.focus.focus(window, cx);
                    app.apply_extension_request(
                        ExtensionUiRequest::Questions(crate::agents::QuestionRequest {
                            id: "option-only".into(),
                            blocking: true,
                            questions: vec![crate::agents::Question {
                                id: "choice".into(),
                                header: "Choice".into(),
                                prompt: "Choose".into(),
                                options: vec![crate::agents::QuestionOption {
                                    label: "A".into(),
                                    description: "First".into(),
                                }],
                                allow_free_text: false,
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
            let panel = cx.update(|_, cx| app.read(cx).overlays.modal.window.expect("modal"));
            let popup_cx = gpui::VisualTestContext::from_window(panel.into(), cx).into_mut();
            popup_cx.update(|window, cx| {
                window.draw(cx).clear(cx);
                assert!(
                    app.read(cx)
                        .extensions
                        .dialog_focus
                        .contains_focused(window, cx)
                );
            });
        },
    );
}
