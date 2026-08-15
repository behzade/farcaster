use super::*;
use serde_json::json;

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
    assert_eq!(state.items[0].text, "done");
    assert_eq!(state.items[0].label, "Bash");
    assert!(!state.items[0].streaming);
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
    assert_eq!(state.items[4].label, "Request User Input");
    assert_eq!(state.items[4].text, "done");
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
