use super::*;

#[test]
fn recorded_greeting_fragments_share_one_text_block() {
    let mut events = Events::default();
    events.message("thinking", &json!({"text":"The user sent a greeting."}));
    events.message("thinking", &json!({"text":"","thinking_duration_ms":498}));
    // Fragments recorded from run-452e4b91-b67d-4b10-8196-6d55619d9bd3.
    for fragment in [
        "Hi", ".", " How", " can", " I", " help", " with", " F", "arc", "aster", " today", "?",
    ] {
        assert_eq!(
            events.message(
                "assistant",
                &json!({"message":{"content":[{"type":"text","text":fragment}]}})
            ),
            vec![WorkerActivity::TextDelta {
                content_index: 1,
                delta: fragment.into(),
            }]
        );
    }
    assert_eq!(events.output, "Hi. How can I help with Farcaster today?");
}

#[test]
fn thinking_fragments_and_tool_boundaries_keep_distinct_blocks() {
    let mut events = Events::default();
    assert_eq!(
        events.message("thinking", &json!({"text":"Let me"})),
        vec![
            WorkerActivity::ThinkingStarted { content_index: 0 },
            WorkerActivity::ThinkingDelta {
                content_index: 0,
                delta: "Let me".into()
            },
        ]
    );
    assert_eq!(
        events.message("thinking", &json!({"text":" think"})),
        vec![WorkerActivity::ThinkingDelta {
            content_index: 0,
            delta: " think".into()
        },]
    );
    for fragment in ["ha", "ha"] {
        assert_eq!(
            events.message(
                "assistant",
                &json!({"message":{"content":[{"type":"text","text":fragment}]}})
            ),
            vec![WorkerActivity::TextDelta {
                content_index: 1,
                delta: fragment.into()
            },]
        );
    }
    events.message(
        "tool_call",
        &json!({"call_id":"c","status":"running","name":"read"}),
    );
    events.message(
        "tool_call",
        &json!({"call_id":"c","status":"completed","result":"done"}),
    );
    assert_eq!(
        events.message(
            "assistant",
            &json!({"message":{"content":[{"type":"text","text":"done"}]}})
        ),
        vec![WorkerActivity::TextDelta {
            content_index: 2,
            delta: "done".into()
        },]
    );
    assert_eq!(events.output, "hahadone");
}

#[test]
fn tool_results_and_string_encoded_counts_survive_translation() {
    let mut events = Events::default();
    let output = events.message("tool_call",&json!({"call_id":"c","status":"error","result":{"content":[{"type":"text","text":"denied"}]}}));
    assert!(
        matches!(output.last().unwrap(),WorkerActivity::ToolFinished {is_error:true,result,..} if result["content"][0]["text"] == "denied")
    );
    assert_eq!(
        usage(&json!({"inputTokens":"12","outputTokens":3,"reasoningTokens":2})).total(),
        15
    );
    assert!(events.message("future", &json!({})).is_empty());
}

#[test]
fn sdk_tool_progress_updates_one_tool_and_respects_step_boundaries() {
    let mut events = Events::default();
    let tool = json!({"type":"shell","args":{"command":"printf hi"}});
    let update = |kind: &str| json!({"type":kind,"callId":"call-1","toolCall":tool});
    assert!(matches!(
        events.interaction(&update("tool-call-started"))[0],
        WorkerActivity::ToolStarted { .. }
    ));
    assert!(matches!(
        events.interaction(&update("partial-tool-call"))[0],
        WorkerActivity::ToolMetadataChanged { .. }
    ));
    let progress =
        json!({"type":"shell-output-delta","event":{"case":"stdout","value":{"data":"hi"}}});
    assert_eq!(
        events.interaction(&progress),
        vec![WorkerActivity::ToolUpdated {
            id: "call-1".into(),
            content: json!([{"type":"text","text":"hi"}])
        }]
    );
    let complete = json!({"type":"tool-call-completed","callId":"call-1","toolCall":{
        "type":"shell","args":tool["args"],"result":{"status":"success","value":{"stdout":"hi","stderr":"failed","exitCode":1}}
    }});
    assert!(
        matches!(events.interaction(&complete).last().unwrap(), WorkerActivity::ToolFinished { is_error:true, result, .. } if result[0]["text"] == "hifailed")
    );
    assert!(events.interaction(&complete).is_empty());
    assert!(events.interaction(&progress).is_empty());
    let text = json!({"type":"text-delta","text":"answer"});
    assert!(matches!(
        events.interaction(&text)[0],
        WorkerActivity::TextDelta {
            content_index: 0,
            ..
        }
    ));
    events.interaction(&json!({"type":"step-completed","stepId":1}));
    assert!(matches!(
        events.interaction(&text)[0],
        WorkerActivity::TextDelta {
            content_index: 1,
            ..
        }
    ));
}

#[test]
fn uncorrelated_shell_output_is_not_assigned_to_a_concurrent_tool() {
    let mut events = Events::default();
    for id in ["one", "two"] {
        events.interaction(
            &json!({"type":"tool-call-started","callId":id,"toolCall":{"type":"shell","args":{}}}),
        );
    }
    assert!(events.interaction(&json!({"type":"shell-output-delta","event":{"case":"stdout","value":{"data":"ambiguous"}}})).is_empty());
}

#[test]
fn a_completion_without_a_start_still_displays_the_tool_and_read_content() {
    let mut events = Events::default();
    let activity = events.interaction(&json!({"type":"tool-call-completed","callId":"read-1","toolCall":{
        "type":"read","args":{"path":"file.rs"},"result":{"status":"success","value":{"content":"fn main() {}","totalLines":1}}
    }}));
    assert!(matches!(&activity[0], WorkerActivity::ToolStarted { id, .. } if id == "read-1"));
    assert!(
        matches!(&activity[1], WorkerActivity::ToolFinished { result, is_error:false, .. } if result[0]["text"] == "fn main() {}")
    );
}
