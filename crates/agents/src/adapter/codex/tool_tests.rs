use super::*;

#[test]
fn generated_images_project_inline_bytes_saved_paths_and_unavailable_ids() {
    let inline = image_output(&json!({"result":"iVBORw0KGgo=","savedPath":"/tmp/output.png"}));
    assert_eq!(inline[0]["type"], "image");
    assert_eq!(inline[0]["mimeType"], "image/png");
    assert_eq!(inline[1]["uri"], "/tmp/output.png");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
    let saved = image_output(&json!({"savedPath":path}));
    assert_eq!(saved[0]["data"], "iVBORw0KGgo=");
    let unavailable = image_output(&json!({"fileId":"provider-only"}));
    assert!(
        unavailable[0]["text"]
            .as_str()
            .unwrap()
            .contains("no local file or image bytes")
    );
    let failed = image_output(&json!({"failure":"generation failed"}));
    assert!(
        failed[0]["text"]
            .as_str()
            .unwrap()
            .contains("generation failed")
    );
}

#[test]
fn command_projection_preserves_native_actions_and_targets() {
    let item = json!({
        "type":"commandExecution",
        "id":"command-1",
        "command":"cat one && rg needle two",
        "commandActions":[
            {"type":"read","path":"one","name":"Read one"},
            {"type":"search","path":"two","query":"needle"}
        ],
        "status":"inProgress"
    });
    let projection = project(&item, "commandExecution");
    assert_eq!(projection.name, "bash");
    assert_eq!(projection.args["command"], item["command"]);
    assert_eq!(projection.args["commandActions"], item["commandActions"]);
    assert_eq!(projection.metadata.category, Some(ToolCategory::Execute));
    assert_eq!(projection.metadata.targets, ["one", "two"]);
    assert_eq!(projection.metadata.native, Some(item));
}

#[test]
fn homogeneous_and_unknown_command_actions_keep_native_categories() {
    let reads = json!({
        "commandActions":[
            {"type":"read","path":"one"},
            {"type":"read","path":"two"}
        ]
    });
    assert_eq!(
        metadata(&reads, "commandExecution").category,
        Some(ToolCategory::Read)
    );
    let unknown = json!({"commandActions":[{"type":"unknown","command":"pwd"}]});
    assert_eq!(
        metadata(&unknown, "commandExecution").category,
        Some(ToolCategory::Execute)
    );
}

#[test]
fn file_change_metadata_keeps_every_target() {
    let item = json!({
        "type":"fileChange",
        "changes":[{"path":"a.rs"},{"path":"b.rs"}]
    });
    let metadata = metadata(&item, "fileChange");
    assert_eq!(metadata.category, Some(ToolCategory::Change));
    assert_eq!(metadata.targets, ["a.rs", "b.rs"]);
    assert_eq!(metadata.native, Some(item));
}

#[test]
fn added_files_project_content_as_patches_without_changing_native_data() {
    for (content, expected) in [
        ("", "@@ -0,0 +0,0 @@\n"),
        ("\n", "@@ -0,0 +1,1 @@\n+\n"),
        ("one\ntwo\n", "@@ -0,0 +1,2 @@\n+one\n+two\n"),
        ("one\r\n", "@@ -0,0 +1,1 @@\n+one\r\n"),
        (
            "+literal\n-literal",
            "@@ -0,0 +1,2 @@\n++literal\n+-literal\n\\ No newline at end of file\n",
        ),
    ] {
        let update = json!({
            "path":"existing.rs", "kind":{"type":"update","move_path":null},
            "diff":"@@ -1 +1 @@\n-old\n+new\n"
        });
        let item = json!({"type":"fileChange", "changes":[
            {"path":"new.rs", "kind":{"type":"add"}, "diff":content},
            update
        ]});
        let projection = project(&item, "fileChange");
        assert_eq!(projection.args["changes"][0]["diff"], expected);
        assert_eq!(projection.args["changes"][1], update);
        assert_eq!(projection.metadata.targets, ["new.rs", "existing.rs"]);
        assert_eq!(projection.metadata.native, Some(item));
    }
}

#[test]
fn sleep_projection_reports_the_wait_duration() {
    let item = json!({
        "type":"sleep",
        "id":"call_jmQp",
        "durationMs":120000
    });
    let projection = project(&item, "sleep");
    assert_eq!(projection.name, "wait");
    assert_eq!(projection.args, json!({"durationMs": 120000}));
    assert_eq!(projection.metadata.title.as_deref(), Some("Waiting 2m"));
}

#[test]
fn mcp_output_recovers_serialized_image_results_and_keeps_order() {
    let image = |data| json!({"type":"image","data":data,"mimeType":"image/png"});
    let inner = json!({"content":[{"type":"text","text":"App screenshot"},image("first"),image("second"),{"type":"resource_link","uri":"file:///tmp/notes.html","name":"Notes"}],"isError":false});
    let item = json!({"result":{"content":[
        {"type":"text","text":"Before"},
        {"type":"text","text":inner.to_string()},
        image("direct"),
        {"type":"text","text":"After"}
    ]}});
    assert_eq!(
        mcp_output(&item),
        vec![
            json!({"type":"text","text":"Before"}),
            json!({"type":"text","text":"App screenshot"}),
            image("first"),
            image("second"),
            json!({"type":"resource_link","uri":"file:///tmp/notes.html","name":"Notes"}),
            image("direct"),
            json!({"type":"text","text":"After"}),
        ]
    );
    assert_eq!(
        item["result"]["content"][1]["text"],
        inner.to_string(),
        "native source stays intact"
    );
}

#[test]
fn mcp_output_preserves_plain_json_malformed_and_unknown_envelopes() {
    for text in [
        "not json",
        "{bad json",
        "[1,2,3]",
        "{\"content\":\"plain document\"}",
        "{\"content\":[{\"type\":\"image\",\"data\":\"bytes\"}]}",
        "{\"content\":[{\"type\":\"unknown\"}]}",
        "{\"content\":[{\"type\":\"text\",\"text\":\"literal\"}],\"other\":true}",
    ] {
        let blocks = json!([{"type":"text","text":text}]);
        assert_eq!(
            mcp_output(&json!({"result":{"content":blocks}})),
            blocks.as_array().unwrap().clone()
        );
    }
    let mut block = json!({"type":"text","text":"deep content"});
    for _ in 0..8 {
        block = json!({"type":"text","text":json!({"content":[block]}).to_string()});
    }
    let output = mcp_output(&json!({"result":{"content":[block]}}));
    assert_eq!(output.len(), 1);
    assert!(
        output[0]["text"].as_str().unwrap().contains("content"),
        "deep wrappers remain inspectable at the depth bound"
    );
}
