use super::*;

#[test]
fn requests_are_scoped_deduplicated_and_routed_with_original_ids() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let identity = registry.issue(
        Path::new("/project"),
        CallerProfile {
            backend: "parent-backend".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    identity.bind("parent-session");
    let parent = registry.resolve(identity.token())?;
    let (responses, receiver) = mpsc::channel();
    let question = WorkerInput {
        id: "same-id".into(),
        prompt: "Which?".into(),
        options: vec!["One".into(), "Two".into()],
        secret: false,
    };
    let child = WorkerParent {
        id: parent.worker_id,
        project: parent.project.clone(),
        child_name: "review".into(),
    };
    let first = registry.request_child_input(&child, question.clone(), responses.clone())?;
    let second = registry.request_child_input(&child, question.clone(), responses.clone())?;
    assert!(
        registry
            .take_child_inputs(&parent.project, "other-backend", "parent-session")
            .is_empty()
    );
    assert!(
        registry
            .take_child_inputs(Path::new("/other"), "parent-backend", "parent-session")
            .is_empty()
    );
    let inputs = registry.take_child_inputs(&parent.project, "parent-backend", "parent-session");
    assert_eq!(inputs.len(), 2);
    assert_ne!(inputs[0].id, inputs[1].id);
    assert!(inputs[0].prompt.contains("review"));
    assert!(
        registry
            .take_child_inputs(&parent.project, "parent-backend", "parent-session")
            .is_empty()
    );
    registry.respond_to_child_input(WorkerInputResponse {
        id: inputs[0].id.clone(),
        value: Some("Two".into()),
        cancel: false,
    })?;
    assert_eq!(
        receiver.try_recv().expect("test operation should succeed"),
        WorkerInputResponse {
            id: "same-id".into(),
            value: Some("Two".into()),
            cancel: false
        }
    );
    registry.respond_to_child_input(WorkerInputResponse {
        id: inputs[1].id.clone(),
        value: None,
        cancel: true,
    })?;
    assert!(
        receiver
            .try_recv()
            .expect("test operation should succeed")
            .cancel
    );
    assert!(
        registry
            .respond_to_child_input(WorkerInputResponse {
                id: inputs[0].id.clone(),
                value: None,
                cancel: true
            })
            .is_err()
    );
    drop((first, second));
    let unanswered = registry.request_child_input(&child, question, responses)?;
    drop(unanswered);
    assert!(
        registry
            .take_child_inputs(&parent.project, "parent-backend", "parent-session")
            .is_empty()
    );
    Ok(())
}
