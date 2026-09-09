use super::*;

#[test]
fn comment_preserves_unsaved_code_and_nested_fences() {
    let context = CodeContext {
        path: "/project/it's code.md".into(),
        cursor_line: 2,
        cursor_column: 1,
        anchor_line: 4,
        anchor_column: 3,
        mode: "v".into(),
        text: "```rust\nسلام\n```".into(),
        modified: true,
    };
    let prompt = context.prompt("  Explain this  ");
    assert!(prompt.starts_with("Explain this\n\nCode context: /project/it's code.md:2:1–4:3"));
    assert!(prompt.contains("buffer has unsaved edits"));
    assert!(prompt.contains("\n````\n```rust\nسلام\n```\n````"));
}
