use super::{
    QueuedMessageKind, choice_copy, composer_primary_action, default_dialog_selection, dialog_copy,
    dialog_number_selection, numbered_dialog_choice, plain_text_html, queued_message_groups,
};
use crate::{app::views::transcript::conversation::QueueState, protocol::ExtensionUiRequest};

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
    };

    let groups = queued_message_groups(&queue);
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0],
        (QueuedMessageKind::Steer, queue.steering.as_slice())
    );
    assert_eq!(
        groups[1],
        (QueuedMessageKind::FollowUp, queue.follow_up.as_slice())
    );
    assert_eq!(groups[0].0.label(), "Steer next");
    assert_eq!(groups[1].0.label(), "Follow-ups");
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
fn select_options_are_numbered_from_one() {
    assert_eq!(numbered_dialog_choice(0, "First"), "1. First");
    assert_eq!(numbered_dialog_choice(4, "Fifth"), "5. Fifth");
}

#[test]
fn enter_defaults_to_the_primary_select_option() {
    let request = ExtensionUiRequest::Select {
        id: "question-1".into(),
        title: "Choose".into(),
        options: vec!["First".into(), "Second".into()],
        timeout: None,
    };

    assert_eq!(
        default_dialog_selection(&request),
        Some(("question-1", "First"))
    );
}

#[test]
fn number_keys_immediately_select_the_matching_first_five_options() {
    let request = ExtensionUiRequest::Select {
        id: "question-1".into(),
        title: "Choose".into(),
        options: (1..=6).map(|number| format!("Option {number}")).collect(),
        timeout: None,
    };

    assert_eq!(
        dialog_number_selection(&request, "1"),
        Some(("question-1", "Option 1"))
    );
    assert_eq!(
        dialog_number_selection(&request, "5"),
        Some(("question-1", "Option 5"))
    );
    assert_eq!(dialog_number_selection(&request, "6"), None);
    assert_eq!(dialog_number_selection(&request, "0"), None);
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

#[test]
fn enter_has_no_default_for_an_empty_select_or_text_input() {
    let empty = ExtensionUiRequest::Select {
        id: "question-1".into(),
        title: "Choose".into(),
        options: Vec::new(),
        timeout: None,
    };
    let input = ExtensionUiRequest::Input {
        id: "question-2".into(),
        title: "Explain".into(),
        placeholder: None,
        timeout: None,
    };

    assert_eq!(default_dialog_selection(&empty), None);
    assert_eq!(default_dialog_selection(&input), None);
}
