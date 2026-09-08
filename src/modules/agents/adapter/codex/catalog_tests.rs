use super::*;

#[test]
fn discovers_previewless_descendants_across_pages_without_duplicates() -> Result<(), String> {
    let home = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let root = json!({"id": "root", "cwd": project, "preview": "Review"});
    let child = json!({"id": "child", "cwd": project, "preview": "",
        "parentThreadId": "root", "status": {"type": "active"}});
    let nested = json!({"id": "nested", "cwd": project, "preview": "",
        "parentThreadId": "child"});
    let responses = [
        json!({"data": [root]}),
        json!({"data": []}),
        json!({"data": []}),
        json!({"data": []}),
        json!({"data": [child], "nextCursor": "page-2"}),
        json!({"data": [child, nested], "nextCursor": null}),
        json!({"data": [], "nextCursor": null}),
    ];
    let input = responses
        .into_iter()
        .enumerate()
        .map(|(index, result)| format!("{}\n", json!({"id": index + 1, "result": result})))
        .collect::<String>();
    let mut requests = Vec::new();
    let mut connection = CodexConnection::new(std::io::Cursor::new(input), &mut requests);
    let sessions = discover_with_client(&mut connection, home.path(), &project, "")?;
    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[1].parent_session.as_deref(), Some("root"));
    assert!(sessions[1].is_running);
    assert_eq!(sessions[2].parent_session.as_deref(), Some("child"));
    let requests = String::from_utf8(requests)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(requests[4]["params"]["ancestorThreadId"], "root");
    assert_eq!(requests[5]["params"]["cursor"], "page-2");
    assert_eq!(requests[6]["params"]["archived"], true);
    Ok(())
}

#[test]
fn native_child_history_keeps_identity_and_outcome() {
    let messages = history_messages(&json!({"type": "subAgentActivity", "id": "activity",
        "kind": "completed", "agentThreadId": "child", "agentPath": "/root/reviewer"}));
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0]["content"][0]["arguments"]["agentThreadId"],
        "child"
    );
    assert_eq!(
        messages[0]["content"][0]["toolMetadata"]["category"],
        "delegate"
    );
    assert_eq!(
        messages[1]["content"][0]["text"],
        "/root/reviewer completed"
    );
}

#[test]
fn owning_connection_supplies_child_status_when_catalog_reports_not_loaded() -> Result<(), String> {
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let thread = json!({"id": "catalog-native-child", "cwd": project,
        "preview": "", "parentThreadId": "catalog-native-parent",
        "source": {"subAgent": {"thread_spawn": {"agent_path": "/root/reviewer"}}},
        "status": {"type": "notLoaded"}});
    for (kind, running) in [("started", true), ("completed", false)] {
        super::super::subagents::observe(
            "catalog-native-parent",
            &json!({
                "agentThreadId": "catalog-native-child", "kind": kind
            }),
        );
        let session = summary(&project, &thread, false)?.ok_or("child")?;
        assert_eq!(session.is_running, running);
        assert_eq!(session.title, "/root/reviewer");
    }
    super::super::subagents::forget_parent("catalog-native-parent");
    Ok(())
}

#[test]
fn translates_thread_metadata() -> Result<(), String> {
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let value = json!({
        "id": "thread-1",
        "cwd": project,
        "name": "Fix tests",
        "preview": "Please fix tests",
        "updatedAt": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs(),
        "status": {"type": "active"},
        "tokenUsage": {"total": {"inputTokens": 100, "outputTokens": 20, "cachedInputTokens": 80}},
    });
    let session = summary(project.as_path(), &value, false)?.ok_or("summary")?;
    assert_eq!(session.harness, "codex-cli");
    assert!(session.is_running);
    assert_eq!(session.title, "Fix tests");
    assert_eq!(session.usage.input, 20);
    assert_eq!(session.usage.output, 20);
    assert_eq!(session.usage.cache_read, 80);
    assert_eq!(session.usage.total, 120);
    Ok(())
}

#[test]
fn skips_auto_review_subsessions() -> Result<(), String> {
    let project = std::env::current_dir().map_err(|error| error.to_string())?;
    let base = json!({
        "id": "thread-1",
        "cwd": project,
        "preview": "reviewing tool call",
    });
    assert!(
        summary(project.as_path(), &base, false)?.is_some(),
        "regular threads are discovered"
    );
    let auto_review = json!({
        "id": "thread-2",
        "cwd": project,
        "model": EPHEMERAL_MODELS[0],
    });
    assert!(
        summary(project.as_path(), &auto_review, false)?.is_none(),
        "auto-review subsessions are not discovered"
    );
    for (source, discovered) in [
        (json!({"subAgent": {"other": "guardian"}}), false),
        (json!({"subAgent": {"other": "custom-worker"}}), true),
        (json!({"subAgent": "review"}), true),
    ] {
        let mut thread = base.clone();
        thread["source"] = source;
        assert_eq!(summary(&project, &thread, false)?.is_some(), discovered);
    }
    Ok(())
}

#[test]
fn discovery_requests_native_agent_sources_explicitly() {
    let params = thread_list_params(false, "", AGENT_SOURCE_KINDS);
    assert_eq!(
        params["sourceKinds"],
        json!([
            "subAgent",
            "subAgentReview",
            "subAgentCompact",
            "subAgentThreadSpawn",
            "subAgentOther"
        ])
    );
    assert!(params["searchTerm"].is_null());
}

#[test]
fn translates_thread_messages() {
    assert_eq!(
        history_messages(&json!({
            "type": "userMessage",
            "content": [{"type": "text", "text": "hello"}],
        }))[0]["role"],
        "user"
    );
    assert_eq!(
        history_messages(&json!({"type": "agentMessage", "text": "done"}))[0]["role"],
        "assistant"
    );
}

#[test]
fn translates_historical_command_calls_and_output() {
    let messages = history_messages(&json!({
        "type": "commandExecution",
        "id": "command-1",
        "command": "cargo test",
        "commandActions": [],
        "status": "completed",
        "aggregatedOutput": "ok",
    }));

    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].pointer("/content/0/type"),
        Some(&json!("toolCall"))
    );
    assert_eq!(messages[0].pointer("/content/0/name"), Some(&json!("bash")));
    assert_eq!(
        messages[0].pointer("/content/0/arguments/command"),
        Some(&json!("cargo test"))
    );
    assert_eq!(
        messages[0].pointer("/content/0/toolMetadata/category"),
        Some(&json!("execute"))
    );
    assert_eq!(
        messages[0].pointer("/content/0/toolMetadata/native"),
        Some(&json!({
            "type": "commandExecution",
            "id": "command-1",
            "command": "cargo test",
            "commandActions": [],
            "status": "completed",
            "aggregatedOutput": "ok",
        }))
    );
    assert_eq!(messages[1]["role"], "toolResult");
    assert_eq!(messages[1].pointer("/content/0/text"), Some(&json!("ok")));
    assert_eq!(messages[1]["isError"], false);
}

#[test]
fn translates_historical_mcp_failures() {
    let messages = history_messages(&json!({
        "type": "mcpToolCall",
        "id": "mcp-1",
        "server": "github",
        "tool": "get_pull_request",
        "arguments": {"number": 42},
        "status": "failed",
        "result": null,
        "error": {"message": "not found"},
    }));

    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].pointer("/content/0/name"),
        Some(&json!("get_pull_request"))
    );
    assert_eq!(
        messages[0].pointer("/content/0/arguments/number"),
        Some(&json!(42))
    );
    assert_eq!(
        messages[1].pointer("/content/0/text"),
        Some(&json!("not found"))
    );
    assert_eq!(messages[1]["isError"], true);
}

#[test]
fn translates_historical_file_changes_and_web_searches() {
    let native_file = json!({
        "type": "fileChange",
        "id": "change-1",
        "changes": [
            {"path": "src/main.rs", "diff": "+fn main() {}"},
            {"path": "src/lib.rs", "diff": "+pub mod new;"}
        ],
        "status": "completed",
    });
    let file = history_messages(&native_file);
    assert_eq!(file[0].pointer("/content/0/name"), Some(&json!("edit")));
    assert_eq!(
        file[0].pointer("/content/0/arguments/path"),
        Some(&json!("src/main.rs"))
    );
    assert_eq!(
        file[0].pointer("/content/0/toolMetadata/targets"),
        Some(&json!(["src/main.rs", "src/lib.rs"]))
    );
    let expected_metadata = serde_json::to_value(tool::metadata(&native_file, "fileChange"))
        .expect("metadata serializes");
    assert_eq!(
        file[0].pointer("/content/0/toolMetadata"),
        Some(&expected_metadata)
    );
    assert_eq!(
        file[1].pointer("/content/0/text"),
        Some(&json!("Applied patch"))
    );

    let search = history_messages(&json!({
        "type": "webSearch",
        "id": "search-1",
        "query": "Codex app-server",
    }));
    assert_eq!(
        search[0].pointer("/content/0/name"),
        Some(&json!("web_search"))
    );
    assert_eq!(
        search[0].pointer("/content/0/arguments/query"),
        Some(&json!("Codex app-server"))
    );
}
