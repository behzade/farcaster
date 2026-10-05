use super::{
    choice_copy, composer_primary_action, dialog_copy, dialog_number_selection,
    numbered_dialog_choice, plain_text_html,
};
use crate::protocol::ExtensionUiRequest;

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
