use super::composer::{choice_copy, dialog_copy};

#[test]
fn permission_dialog_copy_is_short_and_keeps_the_target() {
    let (heading, prompt) =
        dialog_copy("Tool requests an IO right\nAllow bash to access write file /work/file?");
    assert_eq!(heading.as_ref(), "File access request");
    assert_eq!(
        prompt.as_ref().map(AsRef::as_ref),
        Some("Allow bash to write to /work/file?")
    );
}

#[test]
fn permission_choices_use_short_labels_with_clear_scope() {
    let (label, detail) = choice_copy("Always allow in this workspace and retry");
    assert_eq!(label.as_ref(), "Always allow");
    assert_eq!(
        detail.as_ref().map(AsRef::as_ref),
        Some("Remember for this workspace and retry")
    );
    let (label, detail) = choice_copy("No, with comment");
    assert_eq!(label.as_ref(), "Deny with note");
    assert_eq!(
        detail.as_ref().map(AsRef::as_ref),
        Some("Tell Pi what to do instead")
    );
}
