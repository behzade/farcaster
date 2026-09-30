use super::*;

#[test]
fn invocation_tooltip_normalizes_whitespace_and_limits_unicode_characters() {
    let mut state = crate::conversation::ConversationState::default();
    state.reduce_deferred(&serde_json::json!({
        "type":"message_start", "message":{"role":"user", "content":"$example"}
    }));
    let mut item = state.items[0].as_ref().clone();
    item.invocation = Some("  hello\t世界\u{a0}there\n ".into());
    assert_eq!(
        invocation_tooltip_text(&item).as_deref(),
        Some("Prompt expansion: hello 世界 there")
    );
    for (length, suffix) in [(320, ""), (321, "…")] {
        item.invocation = Some("界".repeat(length));
        assert_eq!(
            invocation_tooltip_text(&item),
            Some(format!("Prompt expansion: {}{suffix}", "界".repeat(320)))
        );
    }
}
