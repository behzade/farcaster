use super::*;
use crate::app::views::transcript::conversation::ConversationState;
use serde_json::json;

fn result() -> Value {
    json!({"farcaster_review": {"version": 1, "project": "/project", "review": {
        "title": "Review failure handling", "items": [{"path": "src/main.rs", "start_line": 2, "end_line": 5, "note": "Check retry"}]
    }}})
}

#[test]
fn accepts_structured_and_text_mcp_results_not_arguments() {
    for wrapped in [
        result(),
        json!({"structuredContent": result()}),
        json!({"content": [{"type": "text", "text": result().to_string()}]}),
        Value::String(result().to_string()),
    ] {
        let value = find(&wrapped, 0, &mut 1000).unwrap();
        assert_eq!(value.review.items[0].end_line, Some(5));
    }
    assert!(find(&json!({"arguments": result()}), 0, &mut 1000).is_none());
    let mut invalid = result();
    invalid["farcaster_review"]["review"]["items"][0]["path"] = json!("../escape");
    assert!(find(&invalid, 0, &mut 1000).is_none());
}

#[test]
fn live_tool_result_becomes_review_only_after_success() {
    let mut conversation = ConversationState::default();
    conversation.reduce(&json!({"type":"tool_execution_start", "toolCallId":"review", "toolName":"mcp__farcaster__submit_review", "args":result()}));
    assert!(from_item(&conversation.items[0]).is_none());
    conversation.reduce(&json!({"type":"tool_execution_end", "toolCallId":"review", "result":{"content":[{"type":"text", "text":result().to_string()}]}, "isError":false}));
    assert_eq!(
        from_item(&conversation.items[0]).unwrap().review.title,
        "Review failure handling"
    );
    conversation.reduce(&json!({"type":"tool_execution_start", "toolCallId":"failed", "toolName":"submit_review", "args":{}}));
    conversation.reduce(&json!({"type":"tool_execution_end", "toolCallId":"failed", "result":result(), "isError":true}));
    assert!(from_item(&conversation.items[1]).is_none());
}
