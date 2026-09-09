use super::*;

fn identity(registry: &CallerRegistry, project: &Path, backend: &str) -> CallerIdentity {
    registry.issue(
        project,
        CallerProfile {
            backend: backend.into(),
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
    let caller = identity(&registry, Path::new("/project"), "pi");
    let (id, name) = caller.worker_identity().expect("launch identity");
    assert!(id.starts_with("worker-"));
    assert_ne!(id, caller.token());
    assert!(!name.is_empty());
    caller.bind("native-session");
    let context = context(&registry, &caller);
    assert_eq!((id, name), (context.worker_id, context.worker_name));
}

fn context(registry: &CallerRegistry, identity: &CallerIdentity) -> CallerContext {
    registry
        .resolve(identity.token())
        .expect("registered caller")
}

fn child(
    registry: &CallerRegistry,
    parent: &CallerContext,
    name: &str,
) -> Result<CallerIdentity, String> {
    registry.issue_as(
        &parent.project,
        CallerProfile {
            backend: parent.backend.clone(),
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
    let first = identity(&registry, Path::new("/project"), "pi");
    let second = identity(&registry, Path::new("/project"), "pi");
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
    let identity = identity(&registry, Path::new("/project/two"), "pi");
    identity.bind("session-2");
    identity.select_model("anthropic", "sonnet");
    identity.select_effort("high");

    let resolved = registry.resolve(identity.token()).expect("context");
    assert_eq!(resolved.project, PathBuf::from("/project/two"));
    assert_eq!(resolved.session, "session-2");
    assert_eq!(resolved.backend, "pi");
    assert_eq!(resolved.provider.as_deref(), Some("anthropic"));
    assert_eq!(resolved.model.as_deref(), Some("sonnet"));
    assert_eq!(resolved.effort.as_deref(), Some("high"));
    assert_eq!(resolved.parent_worker_id, None);
    let profile = registry
        .session_profile(Path::new("/project/two"), "pi", "session-2")
        .expect("bound profile");
    assert_eq!(profile.provider, resolved.provider);
    assert_eq!(profile.model, resolved.model);
    assert_eq!(profile.effort, resolved.effort);
    assert!(
        registry
            .session_profile(Path::new("/other"), "pi", "session-2")
            .is_none()
    );
    assert!(
        registry
            .session_profile(Path::new("/project/two"), "codex-cli", "session-2")
            .is_none()
    );
}

#[test]
fn top_level_workers_only_message_their_named_children() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), "pi");
    let unrelated = identity(&registry, Path::new("/project"), "codex-cli");
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
    let parent = identity(&registry, Path::new("/project"), "pi");
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
        registry.session_parent("pi", "child-session").as_deref(),
        Some("parent-session")
    );
    Ok(())
}

#[test]
fn child_names_are_valid_and_unique_within_the_parent() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let first_parent = identity(&registry, Path::new("/project"), "pi");
    let second_parent = identity(&registry, Path::new("/project"), "pi");
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
    let parent = identity(&registry, Path::new("/project"), "pi");
    parent.bind("/sessions/parent.jsonl");
    let context = context(&registry, &parent);
    let child = registry.issue_as(
        &context.project,
        CallerProfile {
            backend: "opencode2".into(),
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
            .native_parent_session(&context.worker_id, "pi")
            .as_deref(),
        Some("/sessions/parent.jsonl")
    );
    assert!(
        registry
            .native_parent_session(&context.worker_id, "opencode2")
            .is_none()
    );
    assert!(
        registry
            .session_parent("opencode2", "opencode-child")
            .is_none()
    );
    assert_eq!(
        links.lock().expect("test operation should succeed").len(),
        1
    );
    assert_eq!(
        links.lock().expect("test operation should succeed")[0].parent_backend,
        "pi"
    );
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
    let parent = identity(&registry, Path::new("/project"), "pi");
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
