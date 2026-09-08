use serde_json::json;

use super::*;

#[test]
fn common_tools_use_shared_names_and_arguments() {
    for (source, canonical) in [
        ("read_file", "read"),
        ("write_file", "write"),
        ("apply_patch", "edit"),
        ("shell", "bash"),
        ("glob", "find"),
        ("list", "ls"),
        ("webfetch", "web_fetch"),
    ] {
        assert_eq!(normalize_opencode_tool(source, &json!({})).0, canonical);
    }
    assert_eq!(
        normalize_opencode_tool(
            "edit",
            &json!({"filePath": "src/main.rs", "oldString": "old", "newString": "new"}),
        )
        .1,
        json!({"path": "src/main.rs", "oldText": "old", "newText": "new"})
    );
}

#[test]
fn metadata_keeps_native_input_and_does_not_guess_custom_intent() {
    let native = json!({
        "state": {
            "input": {"filePath": "src/main.rs"},
            "metadata": {"title": "Inspect source"}
        }
    });
    let metadata = opencode_tool_metadata("read", &json!({"path": "src/main.rs"}), native.clone());
    assert_eq!(metadata.category, Some(ToolCategory::Read));
    assert_eq!(metadata.title.as_deref(), Some("Inspect source"));
    assert_eq!(metadata.targets, ["src/main.rs"]);
    assert_eq!(metadata.native, Some(native));

    let custom = opencode_tool_metadata(
        "mcp_database",
        &json!({"path": "not-a-file-fact"}),
        json!({"progress": 1}),
    );
    assert_eq!(custom.category, Some(ToolCategory::Other));
    assert!(custom.title.is_none());
    assert!(custom.targets.is_empty());
}
