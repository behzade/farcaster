use std::sync::Arc;

use super::*;
use serde_json::json;

#[test]
fn invocation_keeps_one_compact_user_item_through_finalization() {
    let mut state = ConversationState::default();
    state.push_local_user("$commit".into(), 0, true);
    state.start_message(Some(&json!({
        "role":"user",
        "content":[{"type":"text","text":"expanded commit prompt"}]
    })));
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "$commit");
    assert_eq!(state.items[0].invocation.as_deref(), Some(""));

    state.end_message(Some(&json!({
        "role":"user",
        "piUserInvocation":"$commit",
        "content":[{"type":"text","text":"expanded commit prompt"}]
    })));

    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "$commit");
    assert_eq!(
        state.items[0].invocation.as_deref(),
        Some("expanded commit prompt")
    );
}

#[test]
fn farcaster_invocation_keeps_compact_text_without_backend_metadata() {
    let mut state = ConversationState::default();
    state.push_local_invocation("$commit".into(), 0, "expanded commit prompt".into());
    state.start_message(Some(&json!({
        "role":"user",
        "content":[{"type":"text","text":"expanded commit prompt"}]
    })));
    state.end_message(Some(&json!({
        "role":"user",
        "content":[{"type":"text","text":"expanded commit prompt"}]
    })));

    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "$commit");
    assert_eq!(
        state.items[0].invocation.as_deref(),
        Some("expanded commit prompt")
    );
}

#[test]
fn saved_presentations_restore_compact_history_in_order() {
    let mut messages = vec![
        json!({"role":"user","content":"same expansion"}),
        json!({"role":"assistant","content":"one"}),
        json!({"role":"user","content":"same expansion"}),
    ];
    annotate_prompt_presentations(
        &mut messages,
        &[
            crate::agents::PromptPresentation {
                resolved_message: "same expansion".into(),
                display_message: "$commit first".into(),
                invocation: "same expansion".into(),
            },
            crate::agents::PromptPresentation {
                resolved_message: "same expansion".into(),
                display_message: "$commit second".into(),
                invocation: "same expansion".into(),
            },
        ],
    );
    let mut state = ConversationState::default();
    state.replace_history(&messages);

    assert_eq!(state.items[0].text, "$commit first");
    assert_eq!(state.items[1].text, "$commit second");
    assert_eq!(state.items[0].invocation.as_deref(), Some("same expansion"));
}

#[test]
fn pasted_file_contents_stay_out_of_the_transcript() {
    let prompt = "check this\n\nPasted text files:\n- [pasted.txt](</tmp/pasted.txt>)\n\n--- BEGIN PASTED FILE pasted.txt ---\nsecret\n--- END PASTED FILE pasted.txt ---";
    let display = "check this\n\nPasted text files:\n- [pasted.txt](</tmp/pasted.txt>)";
    let mut state = ConversationState::default();

    state.push_local_user(prompt.into(), 0, false);
    assert_eq!(state.items[0].text, display);
    state.start_message(Some(&json!({"role": "user", "content": prompt})));
    state.end_message(Some(&json!({"role": "user", "content": prompt})));

    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, display);

    let mut runtime_state = ConversationState::default();
    runtime_state.push_local_user_with_prompt_images(prompt.into(), &[], false);
    assert_eq!(runtime_state.items[0].text, display);
}

#[test]
fn finalizing_an_optimistic_user_invalidates_its_original_row() {
    let mut state = ConversationState::default();
    state.push_local_user_with_prompt_images("optimistic".into(), &[], false);

    state.reduce(&json!({
        "type": "message_start",
        "message": {"role": "user", "content": "final"}
    }));
    assert_eq!(
        state.reduce(&json!({
            "type": "message_end",
            "message": {"role": "user", "content": "final"}
        })),
        Some(0)
    );
    assert_eq!(state.items[0].text, "final");
}

#[test]
fn optimistic_user_item_rolls_back_by_identity_only() {
    let mut state = ConversationState::default();
    state.push_local_user("same text".into(), 0, false);
    let optimistic = state.push_local_user("same text".into(), 0, false);

    assert!(state.rollback_local_user(&optimistic));
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "same text");
    assert!(!state.rollback_local_user(&optimistic));
}

#[test]
fn cloned_conversations_share_unchanged_transcript_items() {
    let mut state = ConversationState::default();
    state.push_local_user("a long message".repeat(1_000), 0, false);

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
fn updating_one_partial_keeps_other_live_blocks_shared() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    for delta in [
        json!({"type":"thinking_delta","contentIndex":0,"delta":"plan"}),
        json!({"type":"text_delta","contentIndex":1,"delta":"answer"}),
    ] {
        state.reduce(&json!({"type":"message_update","assistantMessageEvent":delta}));
    }
    let thinking = state.items[0].clone();
    let text = state.items[1].clone();

    state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":" continued"}}));

    assert!(Arc::ptr_eq(&thinking, &state.items[0]));
    assert!(!Arc::ptr_eq(&text, &state.items[1]));
    assert_eq!(state.items[1].text, "answer continued");
}

#[test]
fn empty_stream_delta_does_not_reproject_the_live_item() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"answer"}}));
    let item = state.items[0].clone();

    let changed = state.reduce(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":""}}));

    assert_eq!(changed, None);
    assert!(Arc::ptr_eq(&item, &state.items[0]));
}

#[test]
fn live_partial_blocks_remain_sorted_when_indexes_arrive_out_of_order() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    for delta in [
        json!({"type":"toolcall_delta","contentIndex":2,"delta":"tool"}),
        json!({"type":"thinking_delta","contentIndex":0,"delta":"plan"}),
        json!({"type":"text_delta","contentIndex":1,"delta":"answer"}),
    ] {
        state.reduce(&json!({"type":"message_update","assistantMessageEvent":delta}));
    }

    assert_eq!(
        state
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>(),
        ["plan", "answer", "tool"]
    );
}

#[test]
fn deferred_streaming_projects_only_when_the_frame_flushes() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));

    let changed = state.reduce_deferred(&json!({
        "type":"message_update",
        "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"batched"}
    }));
    assert_eq!(changed, Some(0));
    assert!(state.items.is_empty());

    state.flush_live_projection();
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "batched");
}

#[test]
fn long_streams_freeze_chunks_and_keep_only_a_bounded_mutable_tail() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    let source = "abcdefghij".repeat(1_200);
    for part in source.as_bytes().chunks(97) {
        let delta = std::str::from_utf8(part).expect("ASCII fixture");
        state.reduce_deferred(&json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":delta}
        }));
    }
    state.flush_live_projection();

    let item = &state.items[0];
    assert!(item.text.len() <= STREAM_TAIL_MAX_BYTES);
    assert!(!item.stream_chunks.is_empty());
    assert_eq!(item.complete_text(), source);
    let completed = item.stream_chunks[0].clone();

    state.reduce_deferred(&json!({
        "type":"message_update",
        "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"tail"}
    }));
    state.flush_live_projection();
    assert!(Arc::ptr_eq(&completed, &state.items[0].stream_chunks[0]));
    assert_eq!(state.items[0].complete_text(), format!("{source}tail"));
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
fn interrupted_empty_thinking_is_removed_from_live_and_history() {
    let interrupted = json!({
        "role":"assistant",
        "content":[{"type":"thinking","thinking":""}],
        "stopReason":"aborted"
    });

    let mut live = ConversationState::default();
    live.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    live.reduce(&json!({
        "type":"message_update",
        "assistantMessageEvent":{"type":"thinking_start","contentIndex":0}
    }));
    assert_eq!(live.items.len(), 1);
    assert_eq!(live.items[0].kind, TranscriptKind::Thinking);
    live.reduce(&json!({"type":"message_end","message":interrupted.clone()}));

    let mut history = ConversationState::default();
    history.replace_history(&[interrupted]);

    assert!(live.items.is_empty());
    assert!(history.items.is_empty());
}

#[test]
fn cache_hit_rate_uses_all_assistant_prompt_tokens_for_the_session() {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({"role":"assistant","content":[],"usage":{"input":100,"cacheRead":0,"cacheWrite":0}}),
        json!({"role":"assistant","content":[],"usage":{"input":100,"cacheRead":300,"cacheWrite":100}}),
    ]);
    assert_eq!(state.average_cache_hit_rate, Some(50.0));

    state.reduce(&json!({"type":"message_end","message":{
        "role":"assistant",
        "content":[],
        "usage":{"input":200,"cacheRead":700,"cacheWrite":100}
    }}));
    assert_eq!(state.average_cache_hit_rate, Some(62.5));
}

#[test]
fn assistant_error_without_content_is_visible_live_and_in_history() {
    let message = json!({
        "role":"assistant",
        "content":[],
        "stopReason":"error",
        "errorMessage":"400: invalid tool schema"
    });

    let mut live = ConversationState::default();
    live.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    live.reduce(&json!({"type":"message_end","message":message.clone()}));

    let mut history = ConversationState::default();
    history.replace_history(&[message]);

    for state in [live, history] {
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].kind, TranscriptKind::Error);
        assert_eq!(state.items[0].text, "400: invalid tool schema");
        assert!(state.items[0].is_error);
        assert!(!state.items[0].streaming);
    }
}

#[test]
fn assistant_error_with_content_keeps_response_and_appends_error() {
    let failed = json!({
        "role":"assistant",
        "content":[
            {"type":"thinking","thinking":"plan"},
            {"type":"text","text":"partial answer"}
        ],
        "stopReason":"error",
        "errorMessage":"429: overloaded"
    });
    let succeeded = json!({
        "role":"assistant",
        "content":[{"type":"text","text":"retried answer"}],
        "stopReason":"stop"
    });

    let mut live = ConversationState::default();
    live.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    live.reduce(&json!({
        "type":"message_update",
        "assistantMessageEvent":{"type":"text_start","contentIndex":0}
    }));
    live.reduce(&json!({
        "type":"message_update",
        "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"partial answer"}
    }));
    live.reduce(&json!({"type":"message_end","message":failed.clone()}));
    live.reduce(&json!({
        "type":"auto_retry_start",
        "attempt":1,
        "errorMessage":"429: overloaded"
    }));
    live.reduce(&json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
    live.reduce(&json!({"type":"message_end","message":succeeded.clone()}));

    let mut history = ConversationState::default();
    history.replace_history(&[failed, succeeded]);

    for state in [&live, &history] {
        let items = state
            .items
            .iter()
            .filter(|item| item.kind != TranscriptKind::Notice)
            .collect::<Vec<_>>();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].kind, TranscriptKind::Thinking);
        assert_eq!(items[0].text, "plan");
        assert!(!items[0].is_error);
        assert_eq!(items[1].kind, TranscriptKind::Assistant);
        assert_eq!(items[1].text, "partial answer");
        assert!(!items[1].is_error);
        assert_eq!(items[2].kind, TranscriptKind::Error);
        assert_eq!(items[2].text, "429: overloaded");
        assert!(items[2].is_error);
        assert_eq!(items[3].kind, TranscriptKind::Assistant);
        assert_eq!(items[3].text, "retried answer");
        assert!(!items[3].is_error);
    }
}

#[test]
fn assistant_usage_without_prompt_tokens_does_not_change_average_cache_hit_rate() {
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
    assert_eq!(state.average_cache_hit_rate, Some(60.0));
}

#[test]
fn user_images_are_preserved_as_transcript_attachments() {
    let png = "iVBORw0KGgo=";
    let mut state = ConversationState::default();
    state.replace_history(&[json!({
        "role":"user",
        "content":[{"type":"image","data":png,"mimeType":"image/png"}]
    })]);
    assert_eq!(state.items[0].text, "");
    assert_eq!(state.items[0].images.len(), 1);

    let images = [
        PromptImage::new(png.into(), "image/png".into()),
        PromptImage::new(png.into(), "image/png".into()),
    ];
    state.push_local_user_with_prompt_images("look".into(), &images, false);
    assert_eq!(state.items[1].text, "look");
    assert_eq!(state.items[1].images.len(), 2);

    state.replace_history(&[json!({
        "role":"user",
        "piUserInvocation":"$simplify",
        "content":[
            {"type":"text","text":"expanded"},
            {"type":"image","data":png,"mimeType":"image/png"}
        ]
    })]);
    assert_eq!(state.items[0].text, "$simplify");
    assert_eq!(state.items[0].images.len(), 1);
    assert_eq!(state.items[0].invocation.as_deref(), Some("expanded"));
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
fn duplicate_tool_updates_preserve_item_identity() {
    let mut state = ConversationState::default();
    state.reduce(
        &json!({"type":"tool_execution_start","toolCallId":"a","toolName":"bash","args":{}}),
    );
    let update = json!({
        "type":"tool_execution_update",
        "toolCallId":"a",
        "partialResult":{"content":[{"type":"text","text":"one"}]}
    });
    assert_eq!(state.reduce(&update), Some(0));
    let item = state.items[0].clone();

    assert_eq!(state.reduce(&update), None);
    assert!(Arc::ptr_eq(&item, &state.items[0]));
}

#[test]
fn edit_results_update_the_file_change_action() {
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
            "details":{"diff":"- 1 old\n+ 1 new","firstChangedLine":37}
        }),
    ]);

    let action = state.items[0]
        .tool_presentation
        .as_ref()
        .expect("edit action");
    assert_eq!(action.path(), "src/main.rs");
    assert_eq!(action.counts(), (1, 1));
    assert_eq!(action.first_changed_line(), Some(37));
}

#[test]
fn structured_patch_changes_have_edit_counts() {
    let action = tool_presentation(
        "edit",
        &json!({"path":"src/main.rs","changes":[{"diff":"-old\n+new\n+line\n"}]}),
    )
    .expect("edit action");
    assert_eq!(action.path(), "src/main.rs");
    assert_eq!(action.counts(), (2, 1));
}

#[test]
fn write_calls_expose_a_file_change_action() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({"role":"assistant","content":[{
        "type":"toolCall",
        "id":"write-1",
        "name":"write",
        "arguments":{"path":"src/lib.rs","content":"one\ntwo"}
    }]})]);

    let action = state.items[0]
        .tool_presentation
        .as_ref()
        .expect("write action");
    assert_eq!(action.path(), "src/lib.rs");
    assert_eq!(action.counts(), (2, 0));
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
fn subagent_results_keep_custom_context_but_use_dedicated_transcript_rows() {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({
            "role":"custom",
            "customType":"subagent-result",
            "content":"Subagent child-1 (idle) returned:\n# Findings\nlong body",
            "display":true
        }),
        json!({
            "role":"custom",
            "customType":"other-extension",
            "content":"ordinary extension message",
            "display":true
        }),
    ]);

    assert_eq!(state.items[0].kind, TranscriptKind::AgentResult);
    assert_eq!(state.items[0].label, "Subagent result");
    assert!(state.items[0].text.contains("# Findings"));
    assert_eq!(state.items[1].kind, TranscriptKind::Custom);
}

#[test]
fn background_job_results_wake_the_agent_without_becoming_transcript_rows() {
    let mut state = ConversationState::default();
    state.replace_history(&[
        json!({
            "role":"custom",
            "customType":"background-job-result",
            "content":"background output that remains in agent context",
            "display":true
        }),
        json!({
            "role":"assistant",
            "content":[{"type":"text","text":"agent handled the completion"}]
        }),
    ]);

    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].kind, TranscriptKind::Assistant);
    assert_eq!(state.items[0].text, "agent handled the completion");
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
fn host_script_arguments_put_the_command_after_the_reason() {
    let mut state = ConversationState::default();
    state.replace_history(&[json!({
        "role":"assistant",
        "content":[{
            "type":"toolCall",
            "id":"a",
            "name":"request_user_input",
            "arguments":{
                "question":"Need sudo to install docker",
                "script":"sudo apt install docker"
            }
        }]
    })]);
    assert_eq!(
        state.items[0].text,
        "Need sudo to install docker\n\nCommand:\nsudo apt install docker"
    );
    assert_eq!(
        split_command_block(&state.items[0].text),
        Some(("Need sudo to install docker", "sudo apt install docker"))
    );
    assert_eq!(
        split_command_block(
            "Need sudo\n\nWorking directory: /project\n\nCommand:\nsudo apt install docker",
        ),
        Some((
            "Need sudo\n\nWorking directory: /project",
            "sudo apt install docker"
        ))
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

#[test]
fn delivered_user_message_is_removed_from_the_visible_queue() {
    let mut state = ConversationState::default();
    state.reduce(&json!({
        "type": "queue_update",
        "steering": [],
        "followUp": ["do it", "then test"]
    }));

    state.reduce(&json!({
        "type": "message_start",
        "message": {"role": "user", "content": "do it"}
    }));

    assert_eq!(state.queue.follow_up, ["then test"]);
}
