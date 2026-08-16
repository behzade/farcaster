use std::sync::Arc;

use super::*;
use serde_json::json;

#[test]
fn cloned_conversations_share_unchanged_transcript_items() {
    let mut state = ConversationState::default();
    state.push_local_user("a long message".repeat(1_000), 0);

    let cloned = state.clone();

    assert!(Arc::ptr_eq(&state.items[0], &cloned.items[0]));
}

#[test]
fn assembles_ordered_text_thinking_and_tool_arguments_by_index() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    for delta in [
        json!({"type":"thinking_start","contentIndex":0}),
        json!({"type":"thinking_delta","contentIndex":0,"delta":"plan"}),
        json!({"type":"text_start","contentIndex":1}),
        json!({"type":"text_delta","contentIndex":1,"delta":"answer"}),
        json!({"type":"toolcall_start","contentIndex":2}),
        json!({"type":"toolcall_delta","contentIndex":2,"delta":"{\"path\":"}),
        json!({"type":"toolcall_delta","contentIndex":2,"delta":"\"x\"}"}),
    ] {
        state.reduce(&json!({"type":"message_update","assistantMessageEvent":delta}));
    }
    assert_eq!(state.items.len(), 3);
    assert_eq!(state.items[0].kind, TranscriptKind::Thinking);
    assert_eq!(state.items[0].text, "plan");
    assert_eq!(state.items[1].kind, TranscriptKind::Assistant);
    assert_eq!(state.items[1].text, "answer");
    assert_eq!(state.items[2].kind, TranscriptKind::Tool);
    assert_eq!(state.items[2].label, "Tool");
    assert_eq!(state.items[2].text, "{\"path\":\"x\"}");
}

#[test]
fn authoritative_end_replaces_all_partial_blocks_without_duplicate() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_start","contentIndex":0}}));
    state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"draft thought"}}));
    state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"par"}}));
    state.reduce(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"final thought"},{"type":"text","text":"final"},{"type":"toolCall","id":"a","name":"read","arguments":{"path":"x"}}]}}));
    assert_eq!(state.items.len(), 3);
    assert_eq!(state.items[0].text, "final thought");
    assert_eq!(state.items[1].text, "final");
    assert_eq!(state.items[2].label, "Read");
    assert_eq!(state.items[2].text, "Path: x");
    assert!(state.items.iter().all(|item| !item.streaming));
}

#[test]
fn cache_hit_rate_matches_the_latest_assistant_prompt_usage() {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({"role":"assistant","content":[],"usage":{"input":100,"cacheRead":0,"cacheWrite":0}}),
        json!({"role":"assistant","content":[],"usage":{"input":100,"cacheRead":300,"cacheWrite":100}}),
    ]);
    assert_eq!(state.latest_cache_hit_rate, Some(60.0));

    state.reduce(&json!({"type":"message_end","message":{
        "role":"assistant",
        "content":[],
        "usage":{"input":200,"cacheRead":700,"cacheWrite":100}
    }}));
    assert_eq!(state.latest_cache_hit_rate, Some(70.0));
}

#[test]
fn assistant_usage_without_prompt_tokens_clears_the_cache_hit_rate() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({
        "role":"assistant",
        "content":[],
        "usage":{"input":100,"cacheRead":300,"cacheWrite":100}
    })]);
    state.reduce(&json!({"type":"message_end","message":{
        "role":"assistant",
        "content":[],
        "usage":{"input":0,"cacheRead":0,"cacheWrite":0}
    }}));
    assert_eq!(state.latest_cache_hit_rate, None);
}

#[test]
fn user_images_are_visible_even_without_prompt_text() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({
        "role":"user",
        "content":[{"type":"image","data":"a","mimeType":"image/png"}]
    })]);
    assert_eq!(state.items[0].text, "Attached image");

    state.push_local_user("look".into(), 2);
    assert_eq!(state.items[1].text, "look\n\nAttached 2 images");
}

#[test]
fn history_preserves_assistant_block_roles() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({"role":"assistant","content":[
        {"type":"thinking","thinking":"quiet"},
        {"type":"text","text":"visible"},
        {"type":"toolCall","id":"a","name":"bash","arguments":{"command":"true"}}
    ]})]);
    assert_eq!(
        state.items.iter().map(|item| item.kind).collect::<Vec<_>>(),
        [
            TranscriptKind::Thinking,
            TranscriptKind::Assistant,
            TranscriptKind::Tool
        ]
    );
}

#[test]
fn tool_updates_replace_snapshots_and_correlate() {
    let mut state = ConversationState::default();
    state.reduce(
        &json!({"type":"tool_execution_start","toolCallId":"a","toolName":"bash","args":{}}),
    );
    state.reduce(&json!({"type":"tool_execution_update","toolCallId":"a","partialResult":{"content":[{"type":"text","text":"one"}]}}));
    state.reduce(&json!({"type":"tool_execution_update","toolCallId":"a","partialResult":{"content":[{"type":"text","text":"one two"}]}}));
    state.reduce(&json!({"type":"tool_execution_end","toolCallId":"a","result":{"content":[{"type":"text","text":"done"}]},"isError":false}));
    assert_eq!(state.items[0].tool_output, "done");
    assert_eq!(state.items[0].label, "Bash");
    assert!(!state.items[0].streaming);
}

#[test]
fn edit_results_keep_the_structured_diff_for_native_rendering() {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({"role":"assistant","content":[{
            "type":"toolCall",
            "id":"edit-1",
            "name":"edit",
            "arguments":{"path":"src/main.rs","edits":[{"oldText":"old","newText":"new"}]}
        }]}),
        json!({
            "role":"toolResult",
            "toolCallId":"edit-1",
            "toolName":"edit",
            "content":[{"type":"text","text":"done"}],
            "details":{"diff":"- 1 old\n+ 1 new"}
        }),
    ]);

    assert_eq!(
        state.items[0].tool_presentation,
        Some(ToolPresentation::Edit {
            path: "src/main.rs".into(),
            diff: Some("- 1 old\n+ 1 new".into()),
        })
    );
}

#[test]
fn write_calls_keep_content_for_native_rendering() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({"role":"assistant","content":[{
        "type":"toolCall",
        "id":"write-1",
        "name":"write",
        "arguments":{"path":"src/lib.rs","content":"fn main() {}"}
    }]})]);

    assert_eq!(
        state.items[0].tool_presentation,
        Some(ToolPresentation::Write {
            path: "src/lib.rs".into(),
            content: "fn main() {}".into(),
        })
    );
}

#[test]
fn tool_result_messages_update_the_existing_call_without_a_duplicate() {
    let mut state = ConversationState::default();
    state.reduce(
        &json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"a","name":"read","arguments":{"path":"x"}}
        ]}}),
    );
    state.reduce(&json!({"type":"tool_execution_start","toolCallId":"a","toolName":"read","args":{"path":"x"}}));
    state.reduce(&json!({"type":"tool_execution_end","toolCallId":"a","result":{"content":[{"type":"text","text":"live"}]},"isError":false}));
    state.reduce(&json!({"type":"message_start","message":{"role":"toolResult","toolCallId":"a","toolName":"read","content":[{"type":"text","text":"final"}]}}));
    state.reduce(&json!({"type":"message_end","message":{"role":"toolResult","toolCallId":"a","toolName":"read","content":[{"type":"text","text":"final"}]}}));

    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "Path: x");
    assert_eq!(state.items[0].tool_output, "final");
}

#[test]
fn transcript_uses_quiet_speaker_labels_and_readable_tool_names() {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({"role":"user","content":"question"}),
        json!({"role":"assistant","content":[
            {"type":"thinking","thinking":"first line\nmore"},
            {"type":"text","text":"answer"},
            {"type":"toolCall","id":"a","name":"request_user_input","arguments":{}}
        ]}),
        json!({"role":"toolResult","toolCallId":"a","toolName":"request_user_input","content":[{"type":"text","text":"done"}]})
    ]);

    assert_eq!(state.items[0].label, "");
    assert_eq!(state.items[1].label, "");
    assert_eq!(state.items[2].label, "");
    assert_eq!(state.items[3].label, "Request User Input");
    assert_eq!(state.items[3].tool_output, "done");
}

#[test]
fn tool_name_capitalizes_each_underscore_separated_word() {
    assert_eq!(display_tool_name("x_y"), "X Y");
    assert_eq!(display_tool_name("gitHub_lookup"), "GitHub Lookup");
    assert_eq!(display_tool_name(""), "Tool");
}

#[test]
fn tool_arguments_are_readable_fields_instead_of_raw_json() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({"role":"assistant","content":[{
        "type":"toolCall",
        "id":"a",
        "name":"edit_file",
        "arguments":{
            "path":"src/main.rs",
            "dryRun":false,
            "edits":[{"oldText":"before","newText":"after"}]
        }
    }]})]);
    assert_eq!(
        state.items[0].text,
        "Path: src/main.rs\nDry run: No\nEdits:\n  -\n    Old text: before\n    New text: after"
    );
}

#[test]
fn queue_retry_compaction_and_settlement_project_correctly() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"agent_start"}));
    state.reduce(&json!({"type":"queue_update","steering":["a"],"followUp":["b"]}));
    state.reduce(&json!({"type":"agent_end","willRetry":true}));
    assert!(state.running);
    assert!(state.retrying);
    state.reduce(&json!({"type":"compaction_start","reason":"threshold"}));
    assert!(state.compacting);
    state.reduce(&json!({"type":"agent_settled"}));
    assert!(state.settled);
    assert!(!state.running);
    assert_eq!(state.queue.steering, ["a"]);
}
