use super::*;
use crate::Backend;

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
fn native_file_session_keeps_explicit_launch_profile() {
    let registry = CallerRegistry::default();
    let caller = identity(&registry, Path::new("/project"), Backend::Pi);
    caller.set_harness_profile_id(Some("custom-pi".into()));
    caller.bind("/native/sessions/session.jsonl");
    let bound = context(&registry, &caller);
    assert_eq!(bound.harness_profile_id.as_deref(), Some("custom-pi"));
    assert_eq!(bound.session, "/native/sessions/session.jsonl");
    assert!(bound.session_locator.is_none());
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
        Some(Arc::new(move |_, turn| {
            captured.lock().expect("turns").push(turn.clone());
            Ok(84)
        })),
    );
    let identity = identity(&registry, Path::new("/project"), Backend::Cursor);
    identity.bind("native");
    identity.begin_execution(Some("first"));
    let (_, first) = registry
        .resolve_execution(identity.token())
        .expect("first execution");
    assert_eq!(first.session_record, 84);
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
    assert!(crate::valid_worker_name(&first.worker_name));
    assert!(!first.worker_name.starts_with("worker-"));
}

#[test]
fn resolves_session_with_the_project_and_profile_that_launched_it() {
    let registry = CallerRegistry::default();
    let identity = identity(&registry, Path::new("/project/two"), Backend::Pi);
    identity.bind_with_locator("session-2", Some("/project/two/session-2".into()));
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
        .session_caller(&SessionKey::Locator {
            harness: Backend::Pi,
            profile_id: None,
            path: "/project/two/session-2".into(),
        })
        .expect("bound caller");
    assert_eq!(caller.0, resolved.worker_name);
    assert_eq!(caller.1.provider, resolved.provider);
    assert_eq!(caller.1.model, resolved.model);
    assert_eq!(caller.1.effort, resolved.effort);
    assert!(
        registry
            .session_caller(&SessionKey::Locator {
                harness: Backend::Pi,
                profile_id: None,
                path: "/other/session-2".into()
            })
            .is_none()
    );
    assert!(
        registry
            .session_caller(&SessionKey::Locator {
                harness: Backend::Codex,
                profile_id: None,
                path: "/project/two/session-2".into()
            })
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
    child.bind_with_locator("opencode-child", Some("/project/opencode-child".into()));
    child.bind_with_locator("opencode-child", Some("/project/opencode-child".into()));
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
        crate::WorkerAssignment {
            profile: "fast".into(),
            harness_profile_id: None,
            execution: crate::WorkerExecution {
                harness: Backend::OpenCode,
                provider: "opencode-go".into(),
                model: "glm-5.3-flash".into(),
                effort: Some("high".into()),
                service_tier: None,
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
    assert_eq!(
        registry.session_worker_profile(&SessionKey::Locator {
            harness: Backend::OpenCode,
            profile_id: None,
            path: "/project/opencode-child".into()
        }),
        Some("fast".into())
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
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    parent.bind("parent");
    let concurrency = super::super::concurrency::WorkerConcurrency::new(1);
    let slot = concurrency.reserve()?;
    let child =
        child(&registry, &context(&registry, &parent), "review")?.with_slot(Some(slot.clone()));
    child.bind("child");
    slot.release();
    let other = concurrency.reserve()?;
    let activity = crate::subscribe_worker_activity(Path::new("/project"), std::thread::current());
    let revision = activity.revision();
    assert!(registry.child_inboxes_idle(Path::new("/project")));
    registry.send(parent.token(), "review", "first".into())?;
    assert!(activity.revision() > revision);
    assert!(!registry.child_inboxes_idle(Path::new("/project")));
    registry.send(parent.token(), "review", "second".into())?;
    assert!(child.has_pending_messages());
    assert!(
        child.has_pending_messages(),
        "retirement checks preserve the head message"
    );
    assert!(child.try_recv().is_none());
    assert!(child.try_recv().is_none());
    assert!(
        !registry.child_inboxes_idle(Path::new("/project")),
        "capacity-blocked head still owns work"
    );
    drop(other);
    assert_eq!(child.try_recv().expect("first message").message, "first");
    assert!(concurrency.reserve().is_err(), "delivery reserves capacity");
    assert_eq!(child.try_recv().expect("second message").message, "second");
    assert!(child.try_recv().is_none());
    assert!(!child.has_pending_messages());
    assert!(registry.child_inboxes_idle(Path::new("/project")));
    Ok(())
}

#[test]
fn profiled_parent_binding_cannot_take_another_live_family() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let first = identity(&registry, Path::new("/project"), Backend::Codex);
    first.set_harness_profile_id(Some("one".into()));
    first.bind_with_locator("copied-id", Some("/profiles/one/session".into()));
    let first_context = context(&registry, &first);
    let first_child = child(&registry, &first_context, "review")?;
    first_child.bind("first-child");

    let second = identity(&registry, Path::new("/project"), Backend::Codex);
    second.set_harness_profile_id(Some("two".into()));
    second.bind_with_locator("copied-id", Some("/profiles/two/session".into()));
    assert_eq!(
        registry.send(second.token(), "review", "wrong".into())?,
        None
    );
    let second_child = child(&registry, &context(&registry, &second), "review")?;
    second_child.bind("second-child");
    registry.send(first.token(), "review", "first".into())?;
    registry.send(second.token(), "review", "second".into())?;
    assert_eq!(
        first_child.try_recv().expect("first child").message,
        "first"
    );
    assert_eq!(
        second_child.try_recv().expect("second child").message,
        "second"
    );
    registry.send(first_child.token(), "parent", "report".into())?;
    assert_eq!(first.try_recv().expect("first parent").message, "report");
    assert!(second.try_recv().is_none());
    drop(first);
    let replacement = identity(&registry, Path::new("/project"), Backend::Codex);
    replacement.set_harness_profile_id(Some("one".into()));
    replacement.bind_with_locator("copied-id", Some("/profiles/one/session".into()));
    registry.send(first_child.token(), "parent", "resumed".into())?;
    assert_eq!(
        replacement.try_recv().expect("replacement").message,
        "resumed"
    );
    assert!(second.try_recv().is_none());
    Ok(())
}

#[test]
fn live_worker_precedes_same_session_restart_candidate() -> Result<(), String> {
    let registry = CallerRegistry::default();
    let first = identity(&registry, Path::new("/project"), Backend::Pi);
    first.bind("/sessions/same.jsonl");
    let owned = child(&registry, &context(&registry, &first), "review")?;
    owned.bind("child");
    let second = identity(&registry, Path::new("/project"), Backend::Pi);
    second.bind("/sessions/same.jsonl");
    assert_eq!(
        registry.send(second.token(), "review", "wrong".into())?,
        None
    );
    registry.send(owned.token(), "parent", "exact".into())?;
    assert_eq!(
        first.try_recv().expect("exact live parent").message,
        "exact"
    );
    assert!(second.try_recv().is_none());
    let callers = registry.callers.lock().expect("registry");
    let registered = callers.get(first.token()).expect("first");
    let report = WorkerParent {
        id: registered.worker_id.clone(),
        project: registered.project.clone(),
        child_name: "review".into(),
        binding: registered.binding.clone(),
    };
    assert_eq!(
        report.find(&callers).expect("recipient").worker_id,
        registered.worker_id
    );
    Ok(())
}

#[test]
fn binding_promotes_early_children_and_shared_handles_after_app_id_merge() -> Result<(), String> {
    use std::sync::atomic::{AtomicI64, Ordering};
    let registry = CallerRegistry::default();
    let parent = identity(&registry, Path::new("/project"), Backend::Pi);
    let parent_id = parent.worker_identity().expect("early identity").0;
    let early = registry.issue_as(
        Path::new("/project"),
        CallerProfile {
            backend: Backend::Codex,
            provider: None,
            model: None,
            effort: None,
        },
        None,
        new_worker_id(),
        "early".into(),
        Some(parent_id.clone()),
    )?;
    early.bind("child");
    parent.bind("/sessions/parent.jsonl");
    let binding = registry
        .callers
        .lock()
        .expect("registry")
        .get(parent.token())
        .expect("parent")
        .binding
        .clone();
    assert!(matches!(
        binding
            .lock()
            .expect("binding")
            .as_ref()
            .expect("bound")
            .key,
        Some(SessionKey::Locator { .. })
    ));
    let record = Arc::new(AtomicI64::new(41));
    let current = record.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |caller| {
            Ok(if caller.worker_id == parent_id {
                current.load(Ordering::SeqCst)
            } else {
                100
            })
        })),
        None,
    );
    parent.bind("/sessions/parent.jsonl");
    assert!(
        registry
            .session_caller(&SessionKey::App(AppSessionId::new(41).expect("id")))
            .is_some()
    );
    record.store(84, Ordering::SeqCst);
    let current = context(&registry, &parent);
    assert_eq!(current.app_session_id.map(AppSessionId::get), Some(84));
    assert_eq!(
        binding
            .lock()
            .expect("binding")
            .as_ref()
            .expect("bound")
            .key,
        Some(SessionKey::App(AppSessionId::new(84).expect("id")))
    );
    let callers = registry.callers.lock().expect("registry");
    assert_eq!(
        callers
            .get(early.token())
            .expect("child")
            .parent_session
            .as_ref()
            .expect("parent binding")
            .key,
        Some(SessionKey::App(AppSessionId::new(84).expect("id")))
    );
    drop(callers);
    assert!(
        registry
            .session_caller(&SessionKey::App(AppSessionId::new(41).expect("id")))
            .is_none()
    );
    drop(parent);
    let replacement = identity(&registry, Path::new("/project"), Backend::Pi);
    registry.set_execution_sinks(Some(Arc::new(|_| Ok(84))), None);
    replacement.bind("/sessions/parent.jsonl");
    registry.send(early.token(), "parent", "after merge".into())?;
    assert_eq!(
        replacement.try_recv().expect("replacement report").message,
        "after merge"
    );
    Ok(())
}

#[test]
fn retained_report_binding_follows_replacement_then_merge_but_not_another_session()
-> Result<(), String> {
    use std::sync::atomic::{AtomicI64, Ordering};
    let registry = CallerRegistry::default();
    let record = Arc::new(AtomicI64::new(41));
    let current = record.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |caller| {
            Ok(if caller.session == "/sessions/a.jsonl" {
                current.load(Ordering::SeqCst)
            } else {
                99
            })
        })),
        None,
    );
    let first = identity(&registry, Path::new("/project"), Backend::Pi);
    first.bind("/sessions/a.jsonl");
    let first_context = context(&registry, &first);
    let original_binding = registry
        .callers
        .lock()
        .expect("registry")
        .get(first.token())
        .expect("first")
        .binding
        .clone();
    let report = WorkerParent {
        id: first_context.worker_id.clone(),
        project: first_context.project.clone(),
        child_name: "review".into(),
        binding: original_binding.clone(),
    };
    let (responses, _) = mpsc::channel();
    let lease = registry.request_profile_input(
        &first_context,
        crate::WorkerInput {
            id: "question".into(),
            prompt: "Proceed?".into(),
            options: vec![],
            secret: false,
        },
        responses,
    )?;
    drop(first);
    let replacement = identity(&registry, Path::new("/project"), Backend::Pi);
    replacement.bind("/sessions/a.jsonl");
    record.store(84, Ordering::SeqCst);
    let replacement_context = context(&registry, &replacement);
    assert_eq!(
        original_binding
            .lock()
            .expect("binding")
            .as_ref()
            .expect("bound")
            .key,
        Some(SessionKey::App(AppSessionId::new(84).expect("id")))
    );
    let callers = registry.callers.lock().expect("registry");
    assert_eq!(
        report.find(&callers).expect("report recipient").worker_id,
        replacement_context.worker_id
    );
    drop(callers);
    let scope = CallerSession::from_context(&replacement_context).native;
    let inputs = registry.take_child_inputs_for_session(&scope);
    assert_eq!(inputs.len(), 1);
    drop(lease);
    assert_eq!(
        registry.take_expired_child_inputs_for_session(&scope),
        vec![inputs[0].id.clone()]
    );

    let unrelated = identity(&registry, Path::new("/project"), Backend::Pi);
    unrelated.bind("/sessions/a.jsonl");
    unrelated.bind("/sessions/b.jsonl");
    assert_eq!(
        original_binding
            .lock()
            .expect("binding")
            .as_ref()
            .expect("bound")
            .key,
        Some(SessionKey::App(AppSessionId::new(84).expect("id")))
    );
    drop(replacement);
    assert!(
        report
            .find(&registry.callers.lock().expect("registry"))
            .is_none(),
        "unrelated session must not receive old report"
    );
    Ok(())
}

#[test]
fn family_stop_refreshes_merged_bindings_before_the_next_caller_send() {
    let registry = CallerRegistry::default();
    let record = Arc::new(std::sync::atomic::AtomicI64::new(41));
    let record_for_sink = record.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |_| {
            Ok(record_for_sink.load(std::sync::atomic::Ordering::SeqCst))
        })),
        None,
    );
    let project = Path::new("/project");
    let caller = identity(&registry, project, Backend::Codex);
    caller.bind_with_locator("native", Some("/catalog/codex-native".into()));
    let worker = caller.worker_identity().unwrap().0;
    let binding = registry.worker_binding(project, &worker).unwrap();
    assert_eq!(
        binding.lock().unwrap().as_ref().unwrap().key,
        AppSessionId::new(41).map(SessionKey::App)
    );
    record.store(84, std::sync::atomic::Ordering::SeqCst);
    // The stop path uses this refresh without resolving or sending through caller.
    registry.refresh_session_bindings(project).unwrap();
    assert_eq!(
        binding.lock().unwrap().as_ref().unwrap().key,
        AppSessionId::new(84).map(SessionKey::App)
    );
    registry.set_execution_sinks(Some(Arc::new(|_| Err("storage unavailable".into()))), None);
    assert_eq!(
        registry.refresh_session_bindings(project),
        Err("storage unavailable".into())
    );
}

#[path = "caller/inbox_tests.rs"]
mod retained_inbox_tests;
