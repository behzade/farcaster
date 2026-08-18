use super::composer::{choice_copy, dialog_copy};

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
