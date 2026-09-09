use super::*;

#[test]
fn multi_file_edits_keep_independent_net_counts() {
    let mut first = super::super::tests::write_item();
    let details = Arc::make_mut(
        first
            .tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["src/main.rs".into(), "src/other.rs".into()];
    details.arguments = serde_json::json!({"changes":[
        {"path":"src/main.rs", "diff":"@@ -1 +1 @@\n-old\n+middle"},
        {"path":"src/other.rs", "diff":"@@ -0,0 +1,2 @@\n+one\n+two"}
    ]});
    let mut second = first.clone();
    let details = Arc::make_mut(
        second
            .tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["src/main.rs".into()];
    details.arguments = serde_json::json!({"changes":[
        {"path":"/repo/src/main.rs", "diff":"@@ -1 +1 @@\n-middle\n+final"}
    ]});
    let files = collect(
        [(0, &first), (1, &second)].into_iter(),
        Some(Path::new("/repo")),
        None,
    );
    assert_eq!(files[0].counts, Some((1, 1)));
    assert_eq!(files[1].counts, Some((2, 0)));
    assert_eq!(files[0].line, Some(1));
    assert_eq!(files[1].line, Some(1));
}

#[test]
fn repeated_files_keep_history_without_summing_patch_counts() {
    let first = super::super::tests::write_item();
    let mut second = first.clone();
    let details = Arc::make_mut(
        second
            .tool_details
            .as_mut()
            .expect("test operation should succeed"),
    );
    details.metadata.targets = vec!["/repo/src/main.rs".into(), "src/other.rs".into()];
    let files = collect(
        [(7, &first), (9, &second)].into_iter(),
        Some(Path::new("/repo")),
        None,
    );
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].label, "src/main.rs");
    assert_eq!(files[0].last_operation, Some(9));
    assert_eq!(files[0].counts, None);
    assert_eq!(files[1].label, "src/other.rs");
    assert_eq!(files[1].counts, None);
}

#[test]
fn native_multi_file_patch_shows_every_target_without_aggregate_counts() {
    use crate::app::views::transcript::conversation::ConversationState;
    use serde_json::json;
    let mut state = ConversationState::default();
    state.reduce(&json!({
            "type":"tool_execution_start", "toolCallId":"patch", "toolName":"edit",
            "args":{"path":"src/one/mod.rs", "changes":[
                {"path":"src/one/mod.rs","diff":"+one\n-two"},
                {"path":"src/two/mod.rs","diff":"+three"}
            ]},
            "toolMetadata":{"category":"change", "targets":["src/one/mod.rs", "src/two/mod.rs", "/outside/config"]}
        }));
    state.reduce(&json!({"type":"tool_execution_end", "toolCallId":"patch", "isError":false, "result":{"content":[]}}));
    let files = collect(
        [(3, state.items[0].as_ref())].into_iter(),
        Some(Path::new("/repo")),
        None,
    );
    assert_eq!(
        files
            .iter()
            .map(|file| file.label.as_str())
            .collect::<Vec<_>>(),
        ["/outside/config", "src/one/mod.rs", "src/two/mod.rs"]
    );
    assert!(files.iter().all(|file| file.counts.is_none()));
}
