use super::*;

#[test]
fn short_text_and_exact_limits_are_preserved() {
    for text in ["short\noutput".to_owned(), "a".repeat(MAX_BYTES)] {
        let mut preview = ToolPreview::default();
        preview.push_str(&text);
        assert_eq!(preview.finish(), text);
    }
}

#[test]
fn huge_single_line_stops_at_byte_budget_and_ignores_later_writes() {
    let mut preview = ToolPreview::default();
    preview.push_str(&"a".repeat(1_500_000));
    preview.push_str("must not render");
    assert_eq!(preview.finish(), "a".repeat(MAX_BYTES) + TRUNCATION_NOTICE);
}

#[test]
fn line_budget_applies_across_writes() {
    let mut preview = ToolPreview::default();
    for _ in 0..1_000 {
        preview.push_str("x\n");
    }
    assert_eq!(
        preview.finish(),
        "x\n".repeat(MAX_LINES - 1) + "x" + TRUNCATION_NOTICE
    );
}

#[test]
fn truncation_keeps_valid_unicode() {
    let mut preview = ToolPreview::default();
    preview.push_str(&"a".repeat(MAX_BYTES - 1));
    preview.push_str("🦀");
    assert_eq!(
        preview.finish(),
        "a".repeat(MAX_BYTES - 1) + TRUNCATION_NOTICE
    );
}

#[test]
fn json_serialization_stops_at_preview_limit() {
    let value = serde_json::json!({"data": "a".repeat(1_500_000)});
    let mut preview = ToolPreview::default();
    assert!(serde_json::to_writer_pretty(&mut preview, &value).is_err());
    let text = preview.finish();
    assert!(text.starts_with("{\n  \"data\": \""));
    assert_eq!(text.len(), MAX_BYTES + TRUNCATION_NOTICE.len());
}

#[test]
fn short_json_keeps_the_full_inspection_format() {
    let value = serde_json::json!({"command": "echo 🦀", "args": [1, 2]});
    let mut preview = ToolPreview::default();
    serde_json::to_writer_pretty(&mut preview, &value).unwrap();
    assert_eq!(
        preview.finish(),
        serde_json::to_string_pretty(&value).unwrap()
    );
}
