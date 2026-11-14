use super::*;
use serde_json::json;

fn image() -> Value {
    json!({"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"})
}

#[test]
fn assistant_image_only_reply_survives_history_and_message_end() {
    let message = json!({"role":"assistant","content":[image()]});
    for live in [false, true] {
        let mut state = ConversationState::default();
        if live {
            state.reduce(&json!({"type":"message_end","message":message}));
        } else {
            state.replace_history(&[message.clone()]);
        }
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].images.len(), 1);
        assert_eq!(state.items[0].kind, TranscriptKind::Assistant);
    }
}

#[test]
fn tool_images_and_file_links_survive_live_and_history_without_urls_in_text() {
    let content = json!([{"type":"text","text":"Done"}, image(), {"type":"resource_link","name":"Slides","uri":"file:///tmp/a%20b.html"}]);
    let mut live = ConversationState::default();
    live.reduce(
        &json!({"type":"tool_execution_start","toolCallId":"a","toolName":"make","args":{}}),
    );
    live.reduce(&json!({"type":"tool_execution_end","toolCallId":"a","result":{"content":content},"isError":false}));
    let mut history = ConversationState::default();
    history.replace_history(&[
        json!({"role":"assistant","content":[{"type":"toolCall","id":"a","name":"make","arguments":{}}]}),
        json!({"role":"toolResult","toolCallId":"a","content":content}),
    ]);
    for state in [live, history] {
        assert_eq!(state.items.len(), 1);
        let item = &state.items[0];
        assert_eq!(item.images.len(), 1);
        assert_eq!(item.files[0].path, std::path::Path::new("/tmp/a b.html"));
        assert_eq!(item.tool_output, "Done");
        assert!(item.tool_details.as_ref().unwrap().result.is_some());
    }
}

#[test]
fn streamed_image_markdown_keeps_a_single_parse_until_completion() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    let text = format!("![image](data:image/png;base64,{})", "A".repeat(12000));
    for part in text.as_bytes().chunks(300) {
        state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":std::str::from_utf8(part).unwrap()}}));
    }
    assert_eq!(state.items[0].complete_text(), text);
    assert!(state.items[0].stream_chunks.is_empty());
}
