use super::*;
use serde_json::json;

#[test]
fn history_follows_latest_branch_and_preserves_tool_results() {
    let rows = vec![
        json!({"type":"user","uuid":"u","parentUuid":null,"message":{"role":"user","content":"hi"}}),
        json!({"type":"assistant","uuid":"old","parentUuid":"u","message":{"role":"assistant","content":"abandoned"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","message":{"role":"assistant","model":"fixture","content":[{"type":"tool_use","id":"t","name":"Read","input":{"file_path":905}}]}}),
        json!({"type":"user","uuid":"r","parentUuid":"a","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"bad path","is_error":true}]}}),
        json!({"type":"assistant","uuid":"child","parentUuid":null,"isSidechain":true,"message":{"role":"assistant","content":"child"}}),
    ];
    let history = history(&rows, false);
    assert_eq!(history.messages.len(), 3);
    assert_eq!(
        history.messages[1]["content"][0]["arguments"]["file_path"],
        905
    );
    assert_eq!(history.messages[2]["role"], "toolResult");
    assert_eq!(history.model, Some((BACKEND.into(), "fixture".into())));
}

#[test]
fn compaction_relinks_preserved_messages_without_cycles() {
    let mut rows = vec![
        json!({"type":"system","uuid":"anchor","parentUuid":null,"compactMetadata":{"preservedMessages":{"anchorUuid":"anchor","uuids":["u","a"]}}}),
        json!({"type":"user","uuid":"u","parentUuid":null,"message":{"role":"user","content":"question"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","message":{"role":"assistant","content":"answer"}}),
        json!({"type":"user","uuid":"next","parentUuid":"anchor","message":{"role":"user","content":"next"}}),
    ];
    assert_eq!(history(&rows, false).messages.len(), 3);
    rows[0]["parentUuid"] = json!("next");
    assert_eq!(history(&rows, false).messages.len(), 3);
}

#[test]
fn transcript_reader_tolerates_only_an_incomplete_final_record() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transcript.jsonl");
    std::fs::write(&path, "{\"type\":\"user\"}\n{\"type\":").unwrap();
    assert_eq!(read(&path).unwrap().len(), 1);
    std::fs::write(&path, "{broken}\n").unwrap();
    assert!(read(&path).is_err());
}

#[test]
fn discovery_samples_large_transcripts_and_uses_the_direct_backend_identity() {
    let directory = tempfile::tempdir().unwrap();
    let project_dir = directory.path().join("encoded-project");
    std::fs::create_dir(&project_dir).unwrap();
    let id = "00000000-0000-4000-8000-000000000001";
    let path = project_dir.join(format!("{id}.jsonl"));
    let project = std::env::current_dir().unwrap();
    let first = json!({"type":"user","uuid":"u","parentUuid":null,"cwd":project,
        "message":{"role":"user","content":"find this session"}});
    let large = json!({"type":"progress","data":"x".repeat(200_000)});
    let title = json!({"type":"custom-title","customTitle":"Named session"});
    std::fs::write(&path, format!("{first}\n{large}\n{title}\n")).unwrap();
    let rows = summary(&path).unwrap();
    assert_eq!(rows, vec![first, title]);
    let sessions = discover_in(directory.path(), directory.path(), "named").unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].harness, "claude");
    assert_eq!(sessions[0].first_user_message, "find this session");
    assert_eq!(
        external_session_locator(BACKEND, &sessions[0].path),
        Some(id.into())
    );
}

#[test]
fn discovery_includes_native_children_and_reads_sidechain_history() {
    let directory = tempfile::tempdir().unwrap();
    let project_dir = directory.path().join("encoded-project");
    let parent = "00000000-0000-4000-8000-000000000001";
    let children = project_dir.join(parent).join("subagents");
    fs::create_dir_all(&children).unwrap();
    let project = std::env::current_dir().unwrap();
    fs::write(
        project_dir.join(format!("{parent}.jsonl")),
        format!(
            "{}\n",
            json!({
                "type":"user","cwd":project,"message":{"role":"user","content":"Parent"}
            })
        ),
    )
    .unwrap();
    let rows = vec![
        json!({"type":"user","uuid":"u","parentUuid":null,"isSidechain":true,"message":{"role":"user","content":"Inspect source"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","isSidechain":true,"message":{"role":"assistant","model":"child-model","content":"Found it"}}),
    ];
    fs::write(
        children.join("agent-a123.jsonl"),
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    fs::write(
        children.join("agent-a123.meta.json"),
        r#"{"description":"Source review"}"#,
    )
    .unwrap();
    let sessions = discover_in(directory.path(), directory.path(), "source review").unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, format!("{parent}/a123"));
    assert_eq!(sessions[0].parent_session.as_deref(), Some(parent));
    let history = load_history_in(directory.path(), &sessions[0].path).unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(history.messages[1]["content"][0]["text"], "Found it");
    assert!(child_id(parent, "../../outside").is_none());
}
