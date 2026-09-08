use super::*;
use serde_json::json;

#[test]
fn worker_output_uses_only_final_assistant_text() {
    let message = json!({
        "role": "assistant",
        "content": [
            {"type": "thinking", "thinking": "private"},
            {"type": "text", "text": "first"},
            {"type": "text", "text": " second"}
        ]
    });
    assert_eq!(
        final_assistant_text(Some(&message)).as_deref(),
        Some("first second")
    );
    assert_eq!(
        final_assistant_text(Some(&json!({"role":"user","content":"no"}))),
        None
    );
}

#[test]
fn worker_fork_omits_the_active_worker_start_call() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::NamedTempFile::new()?;
    std::fs::write(
        temp.path(),
        concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"session-1\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"cwd\":\"/project\"}\n",
            "{\"type\":\"message\",\"id\":\"user-1\",\"parentId\":null,\"message\":{\"role\":\"user\",\"content\":\"delegate\"}}\n",
            "{\"type\":\"message\",\"id\":\"assistant-1\",\"parentId\":\"user-1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"name\":\"worker_start\"}]}}\n"
        ),
    )?;
    assert_eq!(
        parent_before_worker_call(temp.path())?.as_deref(),
        Some("user-1")
    );
    Ok(())
}

#[test]
fn worker_input_maps_only_interactive_requests() {
    let (input, kind) = worker_input(ExtensionUiRequest::Confirm {
        id: "confirm-1".into(),
        title: "Proceed?".into(),
        message: "This changes files".into(),
        timeout: None,
    })
    .expect("supported interaction")
    .expect("worker input");
    assert_eq!(input.id, "confirm-1");
    assert_eq!(input.options, ["Yes", "No"]);
    assert!(matches!(kind, InputKind::Confirm));
    assert!(
        worker_input(ExtensionUiRequest::Notify {
            id: "notice".into(),
            message: "done".into(),
            tone: crate::agents::extensions::NotifyTone::Info,
        })
        .expect("non-interactive request")
        .is_none()
    );
    assert!(
        worker_input(ExtensionUiRequest::Unknown {
            id: None,
            method: "future_prompt".into(),
        })
        .is_err()
    );
}
