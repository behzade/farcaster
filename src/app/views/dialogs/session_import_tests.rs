use super::collapsed_import_title;

#[test]
fn import_titles_collapse_whitespace_and_mark_nested_sessions() {
    assert_eq!(
        collapsed_import_title(
            "implementation discussion\nto rely on inspect code   and form conclusions.",
            false
        ),
        "implementation discussion to rely on inspect code and form conclusions."
    );
    assert_eq!(
        collapsed_import_title("child worker", true),
        "↳ child worker"
    );
}
