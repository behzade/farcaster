use super::composer::{
    QueuedMessageKind, choice_copy, composer_primary_action, dialog_copy, queued_message_groups,
};
use crate::conversation::QueueState;

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
fn choice_copy_preserves_extension_owned_copy() {
    let (label, detail) = choice_copy("Add to project policy");
    assert_eq!(label.as_ref(), "Add to project policy");
    assert_eq!(detail, None);
}
