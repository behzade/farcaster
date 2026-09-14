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
        assert_eq!(
            normalize_opencode_tool(source, &json!({}), &Value::Null).0,
            canonical
        );
    }
    assert_eq!(
        normalize_opencode_tool(
            "edit",
            &json!({"filePath": "src/main.rs", "oldString": "old", "newString": "new"}),
            &Value::Null,
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

#[test]
fn completed_patch_files_become_shared_edits_in_live_and_restored_tools() {
    let input = json!({"patchText": "*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n*** End Patch"});
    let diff = "--- a.rs\n+++ a.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let files = json!([
        {"file": "a.rs", "patch": format!("Index: a.rs\n===================================================================\n{diff}")},
        {"file": "b.rs", "patch": "@@ -0,0 +1 @@\n+added\n"}
    ]);
    for native in [
        json!({"metadata": {"files": files}}),
        json!({"state": {"metadata": {"files": files}}}),
    ] {
        let (name, args) = normalize_opencode_tool("patch", &input, &native);
        let metadata = opencode_tool_metadata(&name, &args, native.clone());
        assert_eq!(metadata.category, Some(ToolCategory::Change));
        assert_eq!(metadata.targets, ["a.rs", "b.rs"]);
        assert_eq!(metadata.native, Some(native.clone()));
        assert_eq!(args["path"], "a.rs");
        assert_eq!(
            args["changes"],
            json!([
                {"path": "a.rs", "diff": diff},
                {"path": "b.rs", "diff": files[1]["patch"]}
            ])
        );
        assert_eq!(args["patchText"], input["patchText"]);
        assert_eq!(normalize_opencode_tool(&name, &args, &native), (name, args));

        // File-shaped metadata on an unrelated custom tool is not an edit.
        assert_eq!(
            normalize_opencode_tool("mcp_database", &input, &native).1,
            input
        );
    }
}
