use super::{
    QueuedMessageKind, choice_copy, composer_primary_action, dialog_copy, dialog_number_selection,
    numbered_dialog_choice, plain_text_html, queued_message_groups, queued_message_preview,
};
use crate::{conversation::QueueState, protocol::ExtensionUiRequest};

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
                    super::queue::saved_prompt_body(self.id, &self.text)
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
fn queued_messages_are_grouped_by_delivery_behavior() {
    let queue = QueueState {
        steering: vec!["redirect now".into(), "check this first".into()],
        follow_up: vec!["then summarize".into()],
        ..Default::default()
    };

    let groups = queued_message_groups(&queue);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, QueuedMessageKind::Steer);
    assert_eq!(
        groups[0]
            .1
            .iter()
            .map(|message| message.text)
            .collect::<Vec<_>>(),
        queue.steering.iter().collect::<Vec<_>>()
    );
    assert_eq!(groups[1].0, QueuedMessageKind::FollowUp);
    assert_eq!(
        groups[1]
            .1
            .iter()
            .map(|message| message.text)
            .collect::<Vec<_>>(),
        queue.follow_up.iter().collect::<Vec<_>>()
    );
}

#[test]
fn queue_close_button_requires_owner_evidence_for_that_exact_row() {
    let queue = QueueState {
        steering: vec!["same".into(), "same".into()],
        steering_ids: vec!["owned".into(), "inflight".into()],
        follow_up: vec!["later".into()],
        follow_up_ids: vec!["recovered".into()],
        cancellable_ids: vec!["owned".into(), "recovered".into()],
        ..Default::default()
    };
    let groups = queued_message_groups(&queue);
    assert_eq!(groups[0].1[0].id.map(String::as_str), Some("owned"));
    assert!(!groups[0].1[1].cancellable);
    assert_eq!(groups[1].1[0].id.map(String::as_str), Some("recovered"));
}

#[test]
fn queued_message_preview_hides_multiline_payloads() {
    assert_eq!(
        queued_message_preview("inspect this\n\nPasted text files:\n- file.txt"),
        "inspect this…"
    );
}

#[test]
fn receipt_overlay_preserves_duplicate_occurrences_and_live_cancel_actions() {
    let queue = QueueState {
        steering: vec!["2".into(), "2".into()],
        steering_ids: vec!["first".into(), "second".into()],
        cancellable_ids: vec!["first".into()],
        ..Default::default()
    };
    let receipts = ["first", "second"].map(|id| crate::conversation::PendingReceipt {
        id: id.into(),
        text: "2".into(),
        mode: Some(crate::protocol::PromptMode::Steer),
        images: Default::default(),
        unknown: false,
    });
    let groups = super::queue::pending_message_groups(&queue, &receipts, false);
    assert_eq!(groups[0].1.len(), 2);
    assert_eq!(groups[0].1[0].id.map(String::as_str), Some("first"));
    assert!(!groups[0].1[1].cancellable);
    assert!(groups[0].1.iter().all(|row| !row.dismiss));
    let empty = QueueState::default();
    let restored = super::queue::pending_message_groups(&empty, &receipts, true);
    assert_eq!(restored[0].1.len(), 2);
    assert!(
        restored[0]
            .1
            .iter()
            .all(|row| row.dismiss && row.id.is_some())
    );
}

#[test]
fn peer_messages_have_their_own_queue_group_and_preview() {
    let peer = "Message from Farcaster peer worker-7:\n\nreview complete\nwith details".to_owned();
    let queue = QueueState {
        steering: vec![peer.clone(), "redirect now".into()],
        follow_up: Vec::new(),
        ..Default::default()
    };

    let groups = queued_message_groups(&queue);
    assert_eq!(groups[0].0, QueuedMessageKind::Peer);
    assert_eq!(groups[0].1[0].text, &peer);
    assert_eq!(groups[1].0, QueuedMessageKind::Steer);
    assert_eq!(queued_message_preview(&peer), "worker-7: review complete…");
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
