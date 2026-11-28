use super::output_preview;

#[test]
fn output_preview_shows_bounded_tail_and_marks_omissions() {
    assert_eq!(output_preview("\n\n"), "");
    assert_eq!(output_preview("one\n\ntwo\n"), "one\ntwo");
    assert_eq!(
        output_preview("setup\nwarning\n\nerror\nfailed\n"),
        "… warning\nerror\nfailed"
    );
    let preview = output_preview(&"متن".repeat(10_000));
    assert_eq!(preview.chars().count(), 241);
    assert!(preview.ends_with('…'));
}
