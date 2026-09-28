use super::*;
use crate::Backend;

#[test]
fn retired_parent_wakes_and_routes_nested_child_input_with_exact_profile() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let profile = CallerProfile {
        backend: Backend::Pi,
        provider: None,
        model: None,
        effort: None,
    };
    let (armed, ready) = mpsc::channel();
    let (sent, received) = mpsc::channel();
    let actor = std::thread::spawn(move || {
        armed.send(()).unwrap();
        std::thread::park_timeout(std::time::Duration::from_secs(5));
        sent.send(()).unwrap();
    });
    let parent = registry.issue(
        Path::new("/project"),
        profile.clone(),
        Some(actor.thread().clone()),
    );
    parent.set_harness_profile_id(Some("one".into()));
    parent.bind("parent-session");
    let context = registry.resolve(parent.token())?;
    let scope = CallerSession::from_context(&context).native;
    let middle = registry.issue_as_with_access(
        Path::new("/project"),
        profile,
        None,
        "middle".into(),
        "middle".into(),
        Some(context.worker_id),
        crate::HarnessAccessMode::Auto,
    )?;
    middle.bind("middle-session");
    let context = registry.resolve(middle.token())?;
    let child = WorkerParent::new(
        context.worker_id,
        context.project,
        "nested".into(),
        context.session,
    );
    let retained = parent.retain_inbox()?.unwrap();
    let token = parent.token().to_owned();
    drop(parent);
    assert!(registry.resolve(&token).is_err());
    ready.recv().unwrap();
    let (responses, receiver) = mpsc::channel();
    let lease = registry.request_child_input(
        &child,
        WorkerInput {
            id: "native-approval".into(),
            prompt: "Allow?".into(),
            options: vec![],
            secret: false,
        },
        responses,
    )?;
    for profile_id in [None, Some("two".into())] {
        let mut other = scope.clone();
        other.profile_id = profile_id;
        assert!(registry.take_child_inputs_for_session(&other).is_empty());
    }
    received
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("retired actor wake");
    actor.join().unwrap();
    let inputs = registry.take_child_inputs_for_session(&scope);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].prompt.contains("nested"));
    assert!(registry.take_child_inputs_for_session(&scope).is_empty());
    registry.respond_to_child_input(WorkerInputResponse {
        id: inputs[0].id.clone(),
        value: Some("allow".into()),
        cancel: false,
    })?;
    assert_eq!(
        receiver.try_recv().unwrap(),
        WorkerInputResponse {
            id: "native-approval".into(),
            value: Some("allow".into()),
            cancel: false,
        }
    );
    drop((lease, retained));
    Ok(())
}

#[test]
fn profile_choice_uses_the_parent_input_channel() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let identity = registry.issue(
        Path::new("/project"),
        CallerProfile {
            backend: Backend::Codex,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    );
    identity.bind("parent-session");
    let parent = registry.resolve(identity.token())?;
    let (tx, rx) = mpsc::channel();
    let lease = registry.request_profile_input(
        &parent,
        WorkerInput {
            id: "choice".into(),
            prompt: "Choose model".into(),
            options: vec!["One".into()],
            secret: false,
        },
        tx,
    )?;
    let inputs = registry.take_child_inputs(&parent.project, Backend::Codex, "parent-session");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].prompt, "Choose model");
    registry.respond_to_child_input(WorkerInputResponse {
        id: inputs[0].id.clone(),
        value: Some("One".into()),
        cancel: false,
    })?;
    assert_eq!(
        rx.recv()
            .map_err(|error| error.to_string())?
            .value
            .as_deref(),
        Some("One")
    );
    drop(lease);
    Ok(())
}

#[test]
fn requests_are_scoped_deduplicated_and_routed_with_original_ids() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let identity = registry.issue(
        Path::new("/project"),
        CallerProfile {
            backend: Backend::Pi,
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
            .take_child_inputs(&parent.project, Backend::Codex, "parent-session")
            .is_empty()
    );
    assert!(
        registry
            .take_child_inputs(Path::new("/other"), Backend::Pi, "parent-session")
            .is_empty()
    );
    let inputs = registry.take_child_inputs(&parent.project, Backend::Pi, "parent-session");
    assert_eq!(inputs.len(), 2);
    assert_ne!(inputs[0].id, inputs[1].id);
    assert!(inputs[0].prompt.contains("review"));
    assert!(
        registry
            .take_child_inputs(&parent.project, Backend::Pi, "parent-session")
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
            .take_child_inputs(&parent.project, Backend::Pi, "parent-session")
            .is_empty()
    );
    assert!(
        registry
            .take_expired_child_inputs(&parent.project, Backend::Pi, "parent-session")
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
            backend: Backend::Codex,
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
    let shown = registry.take_child_inputs(Path::new("/project"), Backend::Codex, "parent-session");
    assert_eq!(shown.len(), 1);
    drop(parent);
    let replacement = registry.issue(
        Path::new("/project"),
        CallerProfile {
            backend: Backend::Codex,
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
            .take_expired_child_inputs(Path::new("/project"), Backend::Pi, "parent-session")
            .is_empty()
    );
    assert_eq!(
        registry.take_expired_child_inputs(Path::new("/project"), Backend::Codex, "parent-session"),
        vec![shown[0].id.clone()]
    );
    assert!(
        registry
            .take_expired_child_inputs(Path::new("/project"), Backend::Codex, "parent-session")
            .is_empty()
    );
    Ok(())
}

#[test]
fn profile_binding_keeps_input_and_expiry_with_its_parent() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let issue = |profile: &str| {
        let caller = registry.issue(
            Path::new("/project"),
            CallerProfile {
                backend: Backend::Pi,
                provider: None,
                model: None,
                effort: None,
            },
            None,
        );
        caller.set_harness_profile_id(Some(profile.into()));
        caller.bind("/same/native/session.jsonl");
        caller
    };
    let first = issue("one");
    let caller = registry.resolve(first.token())?;
    let scope = CallerSession::from_context(&caller).native;
    let (responses, _) = mpsc::channel();
    let lease = registry.request_profile_input(
        &caller,
        WorkerInput {
            id: "approval".into(),
            prompt: "Choose".into(),
            options: vec![],
            secret: false,
        },
        responses,
    )?;
    let second = issue("two");
    let other = CallerSession::from_context(&registry.resolve(second.token())?).native;
    assert!(registry.take_child_inputs_for_session(&other).is_empty());
    assert!(
        registry
            .take_child_inputs(&scope.project, scope.harness, &scope.id)
            .is_empty()
    );
    let shown = registry.take_child_inputs_for_session(&scope);
    assert_eq!(shown.len(), 1);
    registry.replay_child_inputs_for_session(&other);
    assert!(registry.take_child_inputs_for_session(&scope).is_empty());
    registry.replay_child_inputs_for_session(&scope);
    let replayed = registry.take_child_inputs_for_session(&scope);
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].id, shown[0].id);
    assert!(registry.take_child_inputs_for_session(&scope).is_empty());
    drop(first);
    let _replacement = issue("one");
    drop(lease);
    assert!(
        registry
            .take_expired_child_inputs_for_session(&other)
            .is_empty()
    );
    assert_eq!(
        registry.take_expired_child_inputs_for_session(&scope),
        vec![shown[0].id.clone()]
    );
    registry.replay_child_inputs_for_session(&scope);
    assert!(registry.take_child_inputs_for_session(&scope).is_empty());
    Ok(())
}
