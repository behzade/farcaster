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
    let child = WorkerParent::new(
        parent.worker_id,
        parent.project.clone(),
        "review".into(),
        parent.session.clone(),
    );
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
    assert!(
        registry
            .take_expired_child_inputs(&parent.project, "parent-backend", "parent-session")
            .is_empty(),
        "an input that was never shown needs no UI dismissal"
    );
    Ok(())
}

#[test]
fn delivered_input_expiry_is_drained_by_stable_parent_session() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = registry.issue(
        Path::new("/project"),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    parent.bind("parent-session");
    let context = registry.resolve(parent.token())?;
    let child = WorkerParent::new(
        context.worker_id,
        context.project.clone(),
        "review".into(),
        context.session.clone(),
    );
    let (responses, _) = mpsc::channel();
    let lease = registry.request_child_input(
        &child,
        WorkerInput {
            id: "backend-id".into(),
            prompt: "Which?".into(),
            options: Vec::new(),
            secret: false,
        },
        responses,
    )?;
    let shown = registry.take_child_inputs(Path::new("/project"), "codex-cli", "parent-session");
    assert_eq!(shown.len(), 1);
    drop(parent);
    let replacement = registry.issue(
        Path::new("/project"),
        CallerProfile {
            backend: "codex-cli".into(),
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    replacement.bind("parent-session");
    drop(lease);

    assert!(
        registry
            .take_expired_child_inputs(Path::new("/project"), "other", "parent-session")
            .is_empty()
    );
    assert_eq!(
        registry.take_expired_child_inputs(Path::new("/project"), "codex-cli", "parent-session"),
        vec![shown[0].id.clone()]
    );
    assert!(
        registry
            .take_expired_child_inputs(Path::new("/project"), "codex-cli", "parent-session")
            .is_empty()
    );
    Ok(())
}
