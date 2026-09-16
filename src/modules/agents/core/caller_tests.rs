use super::*;
use crate::agents::Backend;

fn identity(registry: &CallerRegistry, project: &Path, backend: Backend) -> CallerIdentity {
    registry.issue(
        project,
        CallerProfile {
            backend,
            provider: None,
            model: None,
            effort: None,
        },
        None,
    )
}

#[test]
fn process_metadata_identity_is_available_before_session_binding() {
    let registry = CallerRegistry::default();
    let caller = identity(&registry, Path::new("/project"), Backend::Pi);
    let (id, name) = caller.worker_identity().expect("launch identity");
    assert!(id.starts_with("worker-"));
    assert_ne!(id, caller.token());
    assert!(!name.is_empty());
    caller.bind("native-session");
    let context = context(&registry, &caller);
    assert_eq!((id, name), (context.worker_id, context.worker_name));
}

#[test]
fn transient_identity_keeps_its_locator_without_persisting_a_session() {
    let registry = CallerRegistry::default();
    let registrations = Arc::new(Mutex::new(0));
    let captured = registrations.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |_| {
            *captured.lock().expect("registrations") += 1;
            Ok(42)
        })),
        None,
    );

    let caller =
        identity(&registry, Path::new("/project"), Backend::Codex).without_session_persistence();
    caller.bind("ephemeral-title-thread");
    caller.begin_execution(Some("title"));

    assert_eq!(
        context(&registry, &caller).session,
        "ephemeral-title-thread"
    );
    assert_eq!(*registrations.lock().expect("registrations"), 0);
}

fn context(registry: &CallerRegistry, identity: &CallerIdentity) -> CallerContext {
    registry
        .resolve(identity.token())
        .expect("registered caller")
}

#[test]
fn execution_binding_is_captured_before_later_turns_and_cleared_on_rebind() {
    let registry = CallerRegistry::default();
    let turns = Arc::new(Mutex::new(Vec::new()));
    let captured = turns.clone();
    registry.set_execution_sinks(
        Some(Arc::new(|_| Ok(42))),
        Some(Arc::new(move |turn| {
            captured.lock().expect("turns").push(turn.clone());
            Ok(())
        })),
    );
    let identity = identity(&registry, Path::new("/project"), Backend::Cursor);
    identity.bind("native");
    identity.begin_execution(Some("first"));
    let (_, first) = registry
        .resolve_execution(identity.token())
        .expect("first execution");
    identity.set_activity(WorkerActivityState::Working);
    assert_eq!(
        registry
            .resolve_execution(identity.token())
            .expect("unchanged")
            .1,
        first
    );
    identity.begin_execution(Some("first"));
    assert_eq!(
        turns.lock().expect("turns").len(),
        1,
        "receipt replay cannot create another turn"
    );
    identity.begin_execution(Some("second"));
    assert_eq!(first.prompt_id.as_deref(), Some("first"));
    assert_eq!(
        registry
            .resolve_execution(identity.token())
            .expect("second")
            .1
            .prompt_id
            .as_deref(),
        Some("second")
    );
    identity.bind("another-session");
    assert!(registry.resolve_execution(identity.token()).is_err());
}

fn child(
    registry: &CallerRegistry,
    parent: &CallerContext,
    name: &str,
) -> Result<CallerIdentity, String> {
    registry.issue_as(
        &parent.project,
        CallerProfile {
            backend: parent.backend,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        new_worker_id(),
        name.into(),
        Some(parent.worker_id.clone()),
    )
}

#[test]
fn top_level_workers_receive_distinct_human_names() {
    let registry = CallerRegistry::default();
    let first = identity(&registry, Path::new("/project"), Backend::Pi);
    let second = identity(&registry, Path::new("/project"), Backend::Pi);
    first.bind("session-1");
    second.bind("session-2");

    let first = context(&registry, &first);
    let second = context(&registry, &second);
    assert_ne!(first.worker_name, second.worker_name);
    assert!(crate::agents::valid_worker_name(&first.worker_name));
    assert!(!first.worker_name.starts_with("worker-"));
}

#[test]
fn resolves_session_with_the_project_and_profile_that_launched_it() {
    let registry = CallerRegistry::default();
    let identity = identity(&registry, Path::new("/project/two"), Backend::Pi);
    identity.bind("session-2");
    identity.select_model("anthropic", "sonnet");
    identity.select_effort("high");

    let resolved = registry.resolve(identity.token()).expect("context");
    assert_eq!(resolved.project, PathBuf::from("/project/two"));
    assert_eq!(resolved.session, "session-2");
    assert_eq!(resolved.backend, Backend::Pi);
    assert_eq!(resolved.provider.as_deref(), Some("anthropic"));
    assert_eq!(resolved.model.as_deref(), Some("sonnet"));
    assert_eq!(resolved.effort.as_deref(), Some("high"));
    assert_eq!(resolved.parent_worker_id, None);
    let caller = registry
        .session_caller(Path::new("/project/two"), Backend::Pi, "session-2")
        .expect("bound caller");
    assert_eq!(caller.0, resolved.worker_name);
    assert_eq!(caller.1.provider, resolved.provider);
    assert_eq!(caller.1.model, resolved.model);
    assert_eq!(caller.1.effort, resolved.effort);
    assert!(
        registry
            .session_caller(Path::new("/other"), Backend::Pi, "session-2")
            .is_none()
    );
    assert!(
        registry
            .session_caller(Path::new("/project/two"), Backend::Codex, "session-2")
            .is_none()
    );
}

#[test]
fn top_level_workers_only_message_their_named_children() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    let unrelated = identity(&registry, Path::new("/project"), Backend::Codex);
    parent.bind("parent-session");
    unrelated.bind("unrelated-session");
    let parent_context = context(&registry, &parent);
    let child = child(&registry, &parent_context, "diff-review")?;
    child.bind("child-session");
    child.set_activity(WorkerActivityState::Working);
    assert!(!registry.is_child(parent.token())?);
    assert!(registry.is_child(child.token())?);

    assert_eq!(
        registry.send(parent.token(), "missing", "work".into())?,
        None
    );
    assert_eq!(
        registry.send(parent.token(), "DIFF-review", "check this".into())?,
        Some("diff-review".into())
    );
    assert_eq!(
        child.try_recv(),
        Some(PeerMessage {
            from: parent_context.worker_name,
            message: "check this".into(),
        })
    );
    assert!(unrelated.try_recv().is_none());
    Ok(())
}

#[test]
fn children_only_report_to_their_parent() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    parent.bind("parent-session");
    let parent_context = context(&registry, &parent);
    let child = child(&registry, &parent_context, "review")?;
    child.bind("child-session");

    assert_eq!(
        registry.send(child.token(), "ignored", "review done".into())?,
        Some(parent_context.worker_name.clone())
    );
    assert_eq!(
        parent.try_recv(),
        Some(PeerMessage {
            from: "review".into(),
            message: "review done".into(),
        })
    );
    assert_eq!(
        registry
            .session_parent(Backend::Pi, "child-session")
            .as_deref(),
        Some("parent-session")
    );
    Ok(())
}

#[test]
fn children_route_to_the_same_parent_session_after_process_replacement() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Codex);
    parent.bind("same-native-parent");
    let child = child(&registry, &context(&registry, &parent), "review")?;
    child.bind("child-session");
    drop(parent);

    let wrong_backend = identity(&registry, Path::new("/project"), Backend::Pi);
    wrong_backend.bind("same-native-parent");
    let wrong_project = identity(&registry, Path::new("/other"), Backend::Codex);
    wrong_project.bind("same-native-parent");

    let replacement = identity(&registry, Path::new("/project"), Backend::Codex);
    replacement.bind("same-native-parent");
    assert_eq!(
        registry.send(child.token(), "", "finished".into())?,
        Some(context(&registry, &replacement).worker_name.clone())
    );
    assert_eq!(
        replacement.try_recv(),
        Some(PeerMessage {
            from: "review".into(),
            message: "finished".into(),
        })
    );
    registry.send(replacement.token(), "review", "check again".into())?;
    assert_eq!(
        child.try_recv().expect("replacement reaches child").message,
        "check again"
    );
    assert!(wrong_backend.try_recv().is_none());
    assert!(wrong_project.try_recv().is_none());
    Ok(())
}

#[test]
fn child_names_are_valid_and_unique_within_the_parent() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let first_parent = identity(&registry, Path::new("/project"), Backend::Pi);
    let second_parent = identity(&registry, Path::new("/project"), Backend::Pi);
    first_parent.bind("first-parent");
    second_parent.bind("second-parent");
    let first = context(&registry, &first_parent);
    let second = context(&registry, &second_parent);

    let _first_child = child(&registry, &first, "review")?;
    assert!(child(&registry, &first, "REVIEW").is_err());
    assert!(child(&registry, &first, "bad name").is_err());
    assert!(child(&registry, &second, "review").is_ok());
    Ok(())
}

#[test]
fn foreign_parents_keep_farcaster_links_but_not_native_ancestry() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let links = Arc::new(Mutex::new(Vec::new()));
    let captured = links.clone();
    registry.set_family_sink(Some(Arc::new(move |link| {
        captured
            .lock()
            .expect("test operation should succeed")
            .push(link.clone());
        Ok(())
    })));
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    parent.bind("/sessions/parent.jsonl");
    let context = context(&registry, &parent);
    let child = registry.issue_as(
        &context.project,
        CallerProfile {
            backend: Backend::OpenCode,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        new_worker_id(),
        "inspect".into(),
        Some(context.worker_id.clone()),
    )?;
    child.bind("opencode-child");
    child.bind("opencode-child");
    assert_eq!(
        registry
            .native_parent_session(&context.worker_id, Backend::Pi)
            .as_deref(),
        Some("/sessions/parent.jsonl")
    );
    assert!(
        registry
            .native_parent_session(&context.worker_id, Backend::OpenCode)
            .is_none()
    );
    assert!(
        registry
            .session_parent(Backend::OpenCode, "opencode-child")
            .is_none()
    );
    assert_eq!(
        links.lock().expect("test operation should succeed").len(),
        1
    );
    assert_eq!(
        links.lock().expect("test operation should succeed")[0].parent_backend,
        Backend::Pi
    );
    let worker_id = child
        .worker_identity()
        .expect("registered child identity")
        .0;
    registry.set_assignment(
        &worker_id,
        crate::agents::WorkerAssignment {
            profile: "fast".into(),
            execution: crate::agents::WorkerExecution {
                harness: Backend::OpenCode,
                provider: "opencode-go".into(),
                model: "glm-5.3-flash".into(),
                effort: Some("high".into()),
            },
        },
    )?;
    let routed = links
        .lock()
        .expect("test operation should succeed")
        .last()
        .expect("assignment persistence")
        .routing
        .clone()
        .expect("persisted worker routing");
    assert_eq!(routed.name, "inspect");
    assert_eq!(routed.assignment.profile, "fast");
    child.select_model("opencode-go", "glm-5.3-flash");
    child.select_effort("high");
    let saved = links
        .lock()
        .expect("test operation should succeed")
        .last()
        .expect("test operation should succeed")
        .clone();
    let execution = saved.execution.expect("persisted execution");
    assert_eq!(execution.provider, "opencode-go");
    assert_eq!(execution.model, "glm-5.3-flash");
    assert_eq!(execution.effort.as_deref(), Some("high"));
    registry.send(child.token(), "", "done".into())?;
    assert_eq!(
        parent
            .try_recv()
            .expect("test operation should succeed")
            .message,
        "done"
    );
    registry.send(parent.token(), "inspect", "continue".into())?;
    assert_eq!(
        child
            .try_recv()
            .expect("test operation should succeed")
            .message,
        "continue"
    );
    Ok(())
}

#[test]
fn queued_child_message_waits_for_capacity_without_being_lost() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    parent.bind("parent");
    let concurrency = super::super::concurrency::WorkerConcurrency::new(1);
    let slot = concurrency.reserve()?;
    let child =
        child(&registry, &context(&registry, &parent), "review")?.with_slot(Some(slot.clone()));
    child.bind("child");
    slot.release();
    let other = concurrency.reserve()?;
    registry.send(parent.token(), "review", "first".into())?;
    registry.send(parent.token(), "review", "second".into())?;
    assert!(child.try_recv().is_none());
    assert!(child.try_recv().is_none());
    drop(other);
    assert_eq!(child.try_recv().expect("first message").message, "first");
    assert!(concurrency.reserve().is_err(), "delivery reserves capacity");
    assert_eq!(child.try_recv().expect("second message").message, "second");
    assert!(child.try_recv().is_none());
    Ok(())
}
