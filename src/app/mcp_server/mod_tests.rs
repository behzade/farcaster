use super::*;

fn worker_pool(project: &std::path::Path) -> crate::agents::WorkerPool {
    let (factories, default_backend) =
        crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
    crate::agents::WorkerPool::new(factories, default_backend, project.to_owned(), 4)
        .expect("worker pool")
}

#[test]
fn exposes_only_farcaster_tools() {
    let tools = FarcasterMcp::tool_router().list_all();
    let names = tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "worker_notices",
            "worker_send",
            "workgraph_claim",
            "workgraph_complete",
            "workgraph_patch",
            "workgraph_release",
            "workgraph_search"
        ]
    );
    assert!(
        tools
            .iter()
            .filter(|tool| tool.name.starts_with("worker_"))
            .all(|tool| tool.output_schema.is_some())
    );
}

#[test]
fn tool_schemas_follow_the_caller_role() {
    let parent = tools_for_role(false, &crate::agents::WorkerProfiles::default());
    let parent_send = parent
        .iter()
        .find(|tool| tool.name == "worker_send")
        .expect("parent worker_send");
    assert!(parent.iter().any(|tool| tool.name == "worker_notices"));
    assert_eq!(
        parent_send.input_schema.get("required"),
        Some(&serde_json::json!(["to", "message"]))
    );

    let child = tools_for_role(true, &crate::agents::WorkerProfiles::default());
    let child_send = child
        .iter()
        .find(|tool| tool.name == "worker_send")
        .expect("child worker_send");
    assert!(!child.iter().any(|tool| tool.name == "worker_notices"));
    assert_eq!(
        child_send.input_schema.get("required"),
        Some(&serde_json::json!(["message"]))
    );
    assert!(
        child_send.input_schema["properties"]
            .as_object()
            .is_some_and(|properties| !properties.contains_key("to"))
    );
}

#[test]
fn worker_task_schema_tracks_customization_and_empty_definitions() {
    let mut tasks = crate::agents::WorkerProfiles::default();
    tasks.profiles[0].name = "audit".into();
    tasks.profiles.truncate(1);
    let tools = tools_for_role(false, &tasks);
    let send = tools
        .iter()
        .find(|tool| tool.name == "worker_send")
        .expect("test operation should succeed");
    assert_eq!(
        send.input_schema["properties"]["profile"]["enum"],
        serde_json::json!(["audit"])
    );
    assert!(
        send.input_schema["properties"]["profile"]["description"]
            .as_str()
            .expect("test operation should succeed")
            .contains(&format!("audit: {}", tasks.profiles[0].description))
    );
    for name in ["task", "judgment", "effort", "model"] {
        assert!(send.input_schema["properties"].get(name).is_none());
    }
    let child = tools_for_role(true, &tasks);
    let properties = child
        .iter()
        .find(|tool| tool.name == "worker_send")
        .expect("test operation should succeed")
        .input_schema["properties"]
        .as_object()
        .expect("test operation should succeed");
    for name in ["to", "profile", "task", "judgment"] {
        assert!(!properties.contains_key(name));
    }
    tasks.profiles.clear();
    let tools = tools_for_role(false, &tasks);
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "worker_send")
            .expect("test operation should succeed")
            .input_schema["properties"]["profile"],
        serde_json::json!(false)
    );
}

#[tokio::test]
async fn workgraph_rejects_missing_authenticated_caller() {
    let temp = tempfile::tempdir().expect("project");
    let (updates, _) = async_channel::bounded(1);
    let server = FarcasterMcp::new(
        temp.path().join("unused.db"),
        worker_pool(temp.path()),
        updates,
        notices::NoticeBoard::default(),
    );
    let (parts, _) = axum::http::Request::new(()).into_parts();
    let result = server
        .search(
            Parameters(workgraph::SearchParams {
                query: String::new(),
            }),
            Extension(parts),
        )
        .await;
    assert!(matches!(result, Err(error) if error.contains("registered Farcaster caller")));
    assert!(!temp.path().join("unused.db").exists());
}

#[test]
fn workgraph_schemas_do_not_accept_caller_identity() {
    let tools = FarcasterMcp::tool_router().list_all();
    for tool in tools
        .iter()
        .filter(|tool| tool.name.starts_with("workgraph_"))
    {
        let properties = tool.input_schema["properties"]
            .as_object()
            .expect("test operation should succeed");
        for forbidden in ["project", "sessionId", "sessionPath", "next"] {
            assert!(
                !properties.contains_key(forbidden),
                "{} exposes {forbidden}",
                tool.name
            );
        }
        assert_eq!(tool.input_schema["additionalProperties"], false);
        assert!(tool.output_schema.is_some());
    }
}
