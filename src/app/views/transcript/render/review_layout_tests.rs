use super::*;
use conversation::ConversationState;
use serde_json::json;

fn message(state: &mut ConversationState, role: &str, text: &str) {
    let message = json!({"role":role,"content":[{"type":"text","text":text}]});
    state.reduce(&json!({"type":"message_start","message":message}));
    state.reduce(&json!({"type":"message_end","message":message}));
}

fn tool(state: &mut ConversationState, id: &str, review: bool) {
    state.reduce(&json!({"type":"tool_execution_start","toolCallId":id,"toolName":if review {"submit_review"} else {"bash"},"args":{}}));
    let result = if review {
        json!({"farcaster_review":{"version":1,"project":"/project","review":{
            "title":id,"items":[{"path":"src/main.rs","note":"Inspect"}]
        }}})
    } else {
        json!({"content":[{"type":"text","text":"finished command"}]})
    };
    state.reduce(
        &json!({"type":"tool_execution_end","toolCallId":id,"result":result,"isError":false}),
    );
}

fn start() -> ConversationState {
    let mut state = ConversationState::default();
    message(&mut state, "user", "Please implement");
    state.reduce(&json!({"type":"agent_start"}));
    state
}

fn order(rows: &PersistentVec<TranscriptRow>) -> Vec<usize> {
    rows.iter().map(TranscriptRow::item_start).collect()
}

#[test]
fn live_review_stays_inline_then_moves_after_final_response_on_state_only_settlement() {
    let mut state = start();
    tool(&mut state, "review", true);
    message(&mut state, "assistant", "Checking another thing");
    tool(&mut state, "work", false);
    message(&mut state, "assistant", "Final response");
    let live = project_conversation_rows(&state);
    assert_eq!(order(&live), vec![0, 1, 2, 3, 4]);
    assert!(matches!(
        live[1],
        TranscriptRow::Review { working: true, .. }
    ));
    let before = state.clone();
    state.reduce(&json!({"type":"agent_settled"}));
    assert_eq!(before.items, state.items);
    let update = update_conversation_rows(&live, &before, &state, None);
    let settled = update.rows.expect("updated rows");
    assert_eq!(settled, project_conversation_rows(&state));
    assert_eq!(order(&settled), vec![0, 2, 3, 4, 1]);
    assert!(matches!(
        settled[4],
        TranscriptRow::Review { working: false, .. }
    ));
    let copied = copy_transcript_row_range(&state.items, &settled, 3..=4);
    assert!(copied.starts_with("Final response\n\nTool: submit_review"));
    let review_only = copy_transcript_row_range(&state.items, &settled, 4..=4);
    assert!(review_only.starts_with("Tool: submit_review"));
    assert!(!review_only.contains("Final response"));
    assert_eq!(settled[4].disclosure_key(), live[1].disclosure_key());
    assert!(settled[4].same_position(&live[1]));
    assert!(
        update_conversation_rows(&settled, &state, &state, None)
            .rows
            .is_none()
    );
}

#[test]
fn multiple_reviews_remain_separate_and_do_not_cross_into_the_next_turn() {
    let mut state = start();
    tool(&mut state, "first", true);
    tool(&mut state, "second", true);
    message(&mut state, "assistant", "Done");
    state.reduce(&json!({"type":"agent_settled"}));
    message(&mut state, "user", "Next task");
    state.reduce(&json!({"type":"agent_start"}));
    tool(&mut state, "third", true);
    let rows = project_conversation_rows(&state);
    assert_eq!(order(&rows), vec![0, 3, 1, 2, 4, 5]);
    assert!(matches!(
        rows[5],
        TranscriptRow::Review { working: true, .. }
    ));
}

#[test]
fn steering_and_retry_do_not_end_a_live_handoff() {
    let mut state = start();
    tool(&mut state, "review", true);
    message(&mut state, "user", "Also check cancellation");
    tool(&mut state, "followup", false);
    message(&mut state, "assistant", "Finished both");
    state.reduce(&json!({"type":"agent_end","willRetry":true}));
    let rows = project_conversation_rows(&state);
    assert_eq!(order(&rows), vec![0, 1, 2, 3, 4]);
    state.reduce(&json!({"type":"agent_settled"}));
    assert_eq!(
        order(&project_conversation_rows(&state)),
        vec![0, 2, 3, 4, 1]
    );
}

#[test]
fn restored_history_places_reviews_after_chunked_final_responses_without_duplicates() {
    let mut state = ConversationState::default();
    let result = json!({"farcaster_review":{"version":1,"project":"/project","review":{
        "title":"Review","items":[{"path":"src/main.rs","note":"Inspect"}]
    }}});
    state.replace_history(&[
        json!({"role":"user","content":[{"type":"text","text":"Task"}]}),
        json!({"role":"assistant","content":[{"type":"toolCall","id":"r","name":"submit_review","arguments":{}}]}),
        json!({"role":"toolResult","toolCallId":"r","content":[{"type":"text","text":result.to_string()}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"A final response. ".repeat(2000)}]}),
        json!({"role":"user","content":[{"type":"text","text":"Next"}]}),
    ]);
    let rows = project_conversation_rows(&state);
    let review_index = rows
        .position(|row| matches!(row, TranscriptRow::Review { .. }))
        .expect("review row position");
    assert!(matches!(
        rows[review_index - 1],
        TranscriptRow::MessageChunk {
            index: 2,
            last: true,
            ..
        }
    ));
    assert_eq!(rows[review_index + 1].item_start(), 3);
    assert_eq!(
        rows.iter()
            .filter(|row| matches!(row, TranscriptRow::Review { .. }))
            .count(),
        1
    );
}

#[test]
fn no_final_response_leaves_review_at_its_submission_point() {
    let mut state = start();
    tool(&mut state, "review", true);
    state.push_transport_error("Disconnected".into());
    assert_eq!(order(&project_conversation_rows(&state)), vec![0, 1, 2]);
}

#[test]
fn streaming_review_history_projects_only_the_changed_tail() {
    struct Counted<'a> {
        items: &'a PersistentVec<std::sync::Arc<conversation::TranscriptItem>>,
        reads: std::cell::Cell<usize>,
    }
    impl Indexed<std::sync::Arc<conversation::TranscriptItem>> for Counted<'_> {
        fn len(&self) -> usize {
            self.items.len()
        }
        fn get(&self, index: usize) -> Option<&std::sync::Arc<conversation::TranscriptItem>> {
            self.reads.set(self.reads.get() + 1);
            self.items.get(index)
        }
    }
    let mut state = ConversationState::default();
    for index in 0..2000 {
        message(&mut state, "user", "Old prompt");
        state.reduce(&json!({"type":"agent_start"}));
        tool(&mut state, &format!("review-{index}"), true);
        message(&mut state, "assistant", "Completed response");
        state.reduce(&json!({"type":"agent_settled"}));
    }
    message(&mut state, "user", "Current prompt");
    state.reduce(&json!({"type":"agent_start"}));
    tool(&mut state, "current-review", true);
    message(&mut state, "assistant", "Live text");
    let before = state.clone();
    let rows = project_conversation_rows(&before);
    let last = state.items.len() - 1;
    let mut tail = state.items[last].as_ref().clone();
    tail.text.push_str(" delta");
    state.items.set(last, std::sync::Arc::new(tail));
    let counted = Counted {
        items: &state.items,
        reads: std::cell::Cell::new(0),
    };
    let update = update_rows_with_run(
        &rows,
        &before.items,
        &counted,
        Some(last),
        state.active_run_start(),
        &state.completed_runs,
    );
    assert!(
        counted.reads.get() < 40,
        "historical source items were revisited: {}",
        counted.reads.get()
    );
    assert!(update.unchanged_prefix_rows > 5900);
    assert_eq!(
        update.rows.expect("updated rows"),
        project_conversation_rows(&state)
    );
}
