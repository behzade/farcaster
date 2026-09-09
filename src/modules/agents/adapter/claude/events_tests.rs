use super::*;

#[test]
fn per_block_assistant_envelopes_do_not_repeat_streamed_text() {
    let mut events = Events::default();
    events.start();
    for (index, kind, field, value) in [
        (0, "thinking", "thinking", "Checking"),
        (1, "text", "text", "Hello"),
    ] {
        events.message(&json!({"type":"stream_event","event":{"type":"content_block_start","index":index,"content_block":{"type":kind,field:""}}}));
        events.message(&json!({"type":"stream_event","event":{"type":"content_block_delta","index":index,"delta":{"type":format!("{kind}_delta"),field:value}}}));
        let frame = json!({"type":"assistant","message":{"content":[{"type":kind,field:value}]}});
        events.message(&frame);
        events.message(&frame);
    }
    let output = events
        .pending
        .iter()
        .filter_map(|event| match event {
            WorkerEvent::Activity(WorkerActivity::TextDelta { delta, .. }) => Some(delta.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(output, "Hello");
    assert_eq!(events.output, "Hello");
}

#[test]
fn final_stream_usage_updates_context_without_clearing_omitted_counts() {
    let mut events = Events::default();
    for line in include_str!("fixtures/cli-2.1.236.jsonl").lines() {
        events.message(&serde_json::from_str::<Value>(line).unwrap());
    }
    let usage = events
        .pending
        .iter()
        .find_map(|event| match event {
            WorkerEvent::Activity(WorkerActivity::Usage(usage)) => Some(usage),
            _ => None,
        })
        .unwrap();
    assert_eq!(usage.turn.input, 22635);
    assert_eq!(usage.turn.output, 6);
    events.pending.clear();
    events.message(&json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":7}}}));
    events.message(&json!({"type":"result","usage":{}}));
    assert!(events.pending.iter().any(|event| matches!(event,
        WorkerEvent::Activity(WorkerActivity::Usage(usage)) if usage.turn.input == 22635 && usage.turn.output == 7)));
}

#[test]
fn session_usage_reads_cumulative_model_totals_without_double_counting() {
    let mut events = Events::default();
    let frame = json!({"type":"result","usage":{"input_tokens":2,"output_tokens":1},
        "modelUsage":{"main":{"inputTokens":20,"outputTokens":10},"child":{"inputTokens":5,"outputTokens":3}}});
    for _ in 0..2 {
        events.message(&frame);
    }
    assert!(events.pending.iter().all(|event| matches!(event,
        WorkerEvent::Activity(WorkerActivity::Usage(usage)) if usage.session.input == 25 && usage.session.output == 13)));
}
