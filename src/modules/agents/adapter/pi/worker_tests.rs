use super::*;
use serde_json::json;

const PARENT_WITH_WORKER_CALL: &str = concat!(
    "{\"type\":\"session\",\"version\":3,\"id\":\"session-1\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"cwd\":\"/project\"}\n",
    "{\"type\":\"message\",\"id\":\"user-1\",\"parentId\":null,\"message\":{\"role\":\"user\",\"content\":\"delegate\"}}\n",
    "{\"type\":\"message\",\"id\":\"assistant-1\",\"parentId\":\"user-1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"name\":\"worker_start\"}]}}\n"
);

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
    std::fs::write(temp.path(), PARENT_WITH_WORKER_CALL)?;
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

#[test]
fn cancelled_fork_cannot_create_a_worker_on_the_parent_session()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("parent.jsonl");
    std::fs::write(&parent, PARENT_WITH_WORKER_CALL)?;
    let script = temp.path().join("fake-pi.sh");
    std::fs::write(
        &script,
        include_str!("../../../../../tests/fixtures/fake-pi.sh"),
    )?;
    let factory = PiWorkerFactory::new(AgentLaunchConfig::test_script(
        &script,
        vec!["cancelled-fork".into()],
    ));
    let locator = parent.to_string_lossy().into_owned();
    let result = factory.create(WorkerLaunch {
        slot: None,
        worker_id: "cancelled-fork-worker".into(),
        worker_name: "child".into(),
        project: temp.path().to_path_buf(),
        parent_session: locator.clone(),
        parent_worker_id: None,
        context: WorkerContext::Session {
            session_locator: locator,
        },
        provider: None,
        model: None,
        effort: None,
        ephemeral: false,
    });
    match result {
        Err(error) => assert!(error.contains("cancelled"), "{error}"),
        Ok(mut worker) => {
            worker.close()?;
            panic!("cancelled fork created a runnable worker on its parent");
        }
    }
    let requests = std::fs::read_to_string(temp.path().join("fork-requests"))?;
    assert!(requests.contains("\"type\":\"fork\""));
    assert!(!requests.contains("\"type\":\"set_steering_mode\""));
    Ok(())
}
