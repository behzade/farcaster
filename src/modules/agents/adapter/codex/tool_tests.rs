use super::*;

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
