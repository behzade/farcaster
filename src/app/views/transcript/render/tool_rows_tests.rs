use super::*;
use crate::app::views::transcript::conversation::{ConversationState, ToolReview};
use serde_json::json;

pub(super) fn write_item() -> TranscriptItem {
    let mut state = ConversationState::default();
    state.reduce(&json!({
        "type": "tool_execution_start", "toolCallId": "write-1", "toolName": "write",
        "args": {"path": "src/main.rs", "content": "fn main() {}\n"}
    }));
    state.reduce(&json!({
        "type": "tool_execution_end", "toolCallId": "write-1", "isError": false,
        "result": {"content": [{"type": "text", "text": "Wrote src/main.rs"}]}
    }));
    (*state.items[0]).clone()
}

#[test]
fn file_links_use_each_targets_first_changed_line() {
    let mut item = write_item();
    let details = Arc::make_mut(item.tool_details.as_mut().unwrap());
    details.metadata.targets = vec!["src/main.rs".into(), "src/other.rs".into()];
    details.arguments = json!({"changes": [
        {"path": "src/main.rs", "diff": "@@ -37,3 +37,3 @@\n context\n-old\n+new\n tail"},
        {"path": "src/other.rs", "diff": "@@ -80 +80 @@\n-before\n+after"}
    ]});
    assert_eq!(file_target_line(&item, "src/main.rs", None), Some(38));
    assert_eq!(file_target_line(&item, "src/other.rs", None), Some(80));
    assert_eq!(file_target_line(&item, "missing.rs", None), None);
}

#[test]
fn file_links_handle_insertions_deletions_and_missing_positions() {
    for (diff, expected) in [
        ("@@ -0,0 +1,2 @@\n+one\n+two", Some(1)),
        ("@@ -37,2 +36,0 @@\n-one\n-two", Some(37)),
        ("@@ -1 +1 @@\n same\n@@ -9 +9 @@\n-old\n+new", Some(9)),
        ("@@ -1 +1 @@\n same", None),
        ("+new\n-old", None),
    ] {
        let mut item = write_item();
        Arc::make_mut(item.tool_details.as_mut().unwrap()).result =
            Some(json!({"details": {"unifiedDiff": diff}}));
        assert_eq!(
            file_target_line(&item, "src/main.rs", None),
            expected,
            "{diff}"
        );
    }
}

#[test]
fn only_successful_file_changes_open_editor() {
    let mut item = write_item();
    assert!(file_targets(&item).next().is_some());
    item.streaming = true;
    assert!(file_targets(&item).next().is_none());
    item.streaming = false;
    item.is_error = true;
    assert!(file_targets(&item).next().is_none());
    item.is_error = false;
    Arc::make_mut(item.tool_details.as_mut().unwrap())
        .metadata
        .targets = vec![
        "other.rs".into(),
        String::new(),
        "other.rs".into(),
        "last.rs".into(),
    ];
    assert_eq!(
        file_targets(&item).collect::<Vec<_>>(),
        ["other.rs", "last.rs"]
    );
    Arc::make_mut(item.tool_details.as_mut().unwrap())
        .metadata
        .targets
        .clear();
    assert_eq!(file_targets(&item).collect::<Vec<_>>(), ["src/main.rs"]);
    item.tool_presentation = None;
    assert!(file_targets(&item).next().is_none());
}

#[test]
fn approval_replaces_execution_status_instead_of_coexisting() {
    let mut item = write_item();
    item.streaming = true;
    item.tool_review = Some(ToolReview {
        state: ToolReviewState::Reviewing,
        detail: None,
    });
    assert_eq!(item_status(&item), Some(ToolStatus::Reviewing));
    assert!(file_targets(&item).next().is_none());
    item.is_error = true;
    item.tool_review.as_mut().unwrap().state = ToolReviewState::Blocked;
    assert_eq!(item_status(&item), Some(ToolStatus::Rejected));
    assert!(file_targets(&item).next().is_none());
    item.tool_review.as_mut().unwrap().state = ToolReviewState::Approved;
    assert_eq!(item_status(&item), Some(ToolStatus::Failed));
    item.is_error = false;
    assert_eq!(item_status(&item), Some(ToolStatus::Running));
    item.streaming = false;
    assert_eq!(item_status(&item), Some(ToolStatus::Succeeded));
    assert!(file_targets(&item).next().is_some());
}

#[test]
fn activity_summary_counts_calls_not_events_or_claimed_files() {
    let mut state = ConversationState::default();
    for (id, name, metadata) in [
        (
            "a",
            "read",
            json!({"category":"read", "targets":["same.rs"]}),
        ),
        (
            "b",
            "read",
            json!({"category":"read", "targets":["same.rs"]}),
        ),
        ("c", "bash", json!({"category":"execute"})),
    ] {
        state.reduce(&json!({"type":"tool_execution_start", "toolCallId":id, "toolName":name, "args":{}, "toolMetadata":metadata}));
        state.reduce(&json!({"type":"tool_execution_update", "toolCallId":id, "partialResult":{"content":[]}}));
        state.reduce(&json!({"type":"tool_execution_end", "toolCallId":id, "result":{"content":[]}, "isError":false}));
    }
    assert_eq!(
        activity_summary(state.items.iter().map(AsRef::as_ref)),
        "2 reads · 1 command"
    );
    assert_eq!(item_status(&state.items[2]), Some(ToolStatus::Succeeded));
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"custom", "toolName":"mcp_database", "args":{"path":"not-a-file"}}));
    assert_eq!(
        activity_summary(state.items.iter().map(AsRef::as_ref)),
        "2 reads · 1 command · 1 other action"
    );
}

#[test]
fn native_file_targets_are_openable_without_a_diff_preview() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"acp", "toolName":"Inspect", "args":{}, "toolMetadata":{"category":"read", "targets":["src/main.rs"]}}));
    assert!(file_targets(&state.items[0]).next().is_none());
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"acp", "result":{"content":[]}, "isError":false}));
    assert!(state.items[0].tool_presentation.is_none());
    assert!(file_targets(&state.items[0]).next().is_some());
}

#[test]
fn details_preserve_readable_output_and_do_not_repeat_commands() {
    let mut state = ConversationState::default();
    state.reduce(&json!({"type":"tool_execution_start", "toolCallId":"shell", "toolName":"bash", "args":{"command":"cargo check"}}));
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"shell", "isError":true, "result":{"content":[{"type":"text","text":"error: missing trait\n  src/main.rs:5"}]}}));
    let detail = tool_body_text(&state.items[0]);
    assert_eq!(detail.matches("cargo check").count(), 1);
    assert!(detail.contains("error: missing trait\n  src/main.rs:5"));
}
