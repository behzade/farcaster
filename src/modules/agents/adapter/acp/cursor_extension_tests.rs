use super::*;

#[test]
fn synthetic_image_has_change_metadata_and_target() {
    let (started, _) = notification(
        "cursor/generate_image",
        &json!({
            "toolCallId":"image-1",
            "description":"Create cover art",
            "filePath":"art/cover.png"
        }),
    )
    .expect("test operation should succeed");
    let WorkerEvent::Activity(WorkerActivity::ToolStarted { metadata, .. }) = started else {
        panic!("expected tool start");
    };
    assert_eq!(metadata.category, Some(ToolCategory::Change));
    assert_eq!(metadata.title.as_deref(), Some("Create cover art"));
    assert_eq!(metadata.targets, ["art/cover.png"]);
}
