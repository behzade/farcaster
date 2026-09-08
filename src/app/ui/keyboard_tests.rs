use super::copy_text;

#[test]
fn transcript_mouse_selection_takes_copy_precedence() {
    assert_eq!(
        copy_text(Some("transcript".to_owned()), "composer".to_owned()),
        Some("transcript".to_owned())
    );
}

#[test]
fn copy_preserves_selected_whitespace() {
    assert_eq!(
        copy_text(Some(" \n\t".to_owned()), "composer".to_owned()),
        Some(" \n\t".to_owned())
    );
}

#[test]
fn composer_selection_is_the_input_mode_fallback() {
    assert_eq!(
        copy_text(None, "composer".to_owned()),
        Some("composer".to_owned())
    );
    assert_eq!(copy_text(None, String::new()), None);
}
