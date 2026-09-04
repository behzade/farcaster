use std::{
    fs,
    os::unix::fs::symlink,
    time::{Duration, Instant, SystemTime},
};

use tempfile::tempdir;

use super::*;

fn test_event_channel() -> (SessionEventSender, mpsc::Receiver<RuntimeEvent>) {
    let (sender, receiver) = mpsc::channel();
    (
        SessionEventSender {
            sender,
            supervisor: thread::current(),
        },
        receiver,
    )
}

#[test]
fn runtime_failure_summary_prefers_the_child_error() {
    let failure = "Pi closed stdout (exit code 1). Stderr:\n\
            Warning: settings were unavailable\n\
            Error: Failed to load extension codex-web-search-core.ts\n\
            Hint: Start without extensions";

    assert_eq!(
        failure_summary(failure),
        "Failed to load extension codex-web-search-core.ts"
    );
}

#[test]
fn runtime_failure_details_remove_controls_and_bound_output() {
    let failure = format!("bad\u{1b}value {}", "x".repeat(MAX_FAILURE_DETAILS_CHARS));
    let details = failure_details(&failure);

    assert!(!details.contains('\u{1b}'));
    assert!(details.ends_with('…'));
    assert_eq!(details.chars().count(), MAX_FAILURE_DETAILS_CHARS + 1);
}

#[test]
fn rpc_owned_paths_exclude_history_only_documents() {
    let active = PathBuf::from("/sessions/active.jsonl");
    let background = PathBuf::from("/sessions/background.jsonl");
    let history = PathBuf::from("/sessions/history.jsonl");
    let snapshot = |live: &PathBuf, selected: &PathBuf, history_preview| {
        Arc::new(RuntimeSnapshot {
            connected: true,
            live_session: Some(live.clone()),
            selected_session: Some(selected.clone()),
            history_preview,
            ..RuntimeSnapshot::default()
        })
    };
    let latest = HashMap::from([
        ("active".into(), snapshot(&active, &active, false)),
        ("preview".into(), snapshot(&background, &history, true)),
        ("history".into(), snapshot(&history, &history, true)),
    ]);

    assert_eq!(
        rpc_owned_session_paths(&latest),
        HashSet::from([active, background])
    );
}

#[test]
fn dropping_runtime_waits_for_owned_pi_processes_to_handle_exit() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;
    let marker = temp.path().join("terminated");
    let runtime = RuntimeHandle::spawn_with(
        temp.path().to_path_buf(),
        "shutdown-test".into(),
        None,
        AgentLaunchConfig::test_script(
            &script,
            vec!["term-marker".into(), marker.to_string_lossy().into_owned()],
        ),
    );
    runtime.send(RuntimeCommand::Reload)?;
    // Pi permits up to 15 seconds for its readiness handshake; leave headroom for
    // build-machine load.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut connected = false;
    while Instant::now() < deadline && !connected {
        connected = matches!(
            runtime.try_recv(),
            Ok(RuntimeEvent::Snapshot { snapshot, .. }) if snapshot.connected
        );
        if !connected {
            thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(connected, "owned Pi process did not start");

    drop(runtime);

    assert_eq!(
        fs::read_to_string(&marker).map_err(|error| error.to_string())?,
        "terminated\n",
        "runtime returned before Pi handled application exit"
    );
    Ok(())
}

fn owner_without_process(
    project: PathBuf,
) -> (
    RuntimeOwner,
    mpsc::Receiver<RuntimeEvent>,
    mpsc::Receiver<DiscoveryResult>,
) {
    let (event_tx, event_rx) = test_event_channel();
    let (discovery_tx, discovery_rx) = mpsc::channel();
    let (history_tx, _history_rx) = mpsc::channel();
    (
        RuntimeOwner {
            project: project.clone(),
            harness: "pi".into(),
            session_id: None,
            process_command: AgentLaunchConfig {
                program: PathBuf::from("/definitely/missing/farcaster-test-command"),
                prefix_args: Vec::new(),
                access_mode: HarnessAccessMode::Sandboxed,
                app_proxy: None,
                session_locator_root: None,
            },
            process: None,
            snapshot: RuntimeSnapshot {
                connected: true,
                status: "Ready".into(),
                project,
                ..RuntimeSnapshot::default()
            },
            owns_session_catalog: true,
            session_generation: 0,
            session_discovery_in_flight: false,
            session_refresh_pending: false,
            session_refresh_due: None,
            process_generation: 1,
            pending_prompt_id: None,
            pending_prompt_target: None,
            pending_prompt_item: None,
            pending_outbox_id: None,
            title_generation: SessionTitleGeneration::default(),
            transcript_changed_from: None,
            event_tx,
            discovery_tx,
            history_tx,
            history_generation: 0,
            history_selection_generation: None,
            document_refresh_generation: None,
            pending_document_refresh: None,
            active_session: None,
            parked_snapshot: None,
            deferred_prompt: None,
            pending_session_controls: PendingSessionControls::default(),
            access_mode_changes: AccessModeChangeState::default(),
            startup_state_loaded: false,
            startup_history_loaded: false,
            state: None,
            session_query: String::new(),
        },
        event_rx,
        discovery_rx,
    )
}

fn preview_history(owner: &mut RuntimeOwner, session: PathBuf, message: &str) {
    owner.snapshot.history_preview = true;
    owner.snapshot.selected_session = Some(session);
    conversation_mut(&mut owner.snapshot).replace_history(&[json!({
        "role": "user",
        "content": message,
        "timestamp": 1
    })]);
    owner.parked_snapshot = Some(RuntimeSnapshot::default());
}

fn drive_process_until(owner: &mut RuntimeOwner, ready: impl Fn(&RuntimeOwner) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        while let Some(item) = owner.process.as_mut().and_then(|process| process.poll()) {
            owner.apply_process_item(item);
        }
        if ready(owner) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn access_mode_before_connection_configures_the_first_process() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["quiet".into()]);
    let target = HarnessAccessMode::Full;

    owner.apply_command(RuntimeCommand::SetAccessMode(target));

    assert!(owner.process.is_none());
    assert_eq!(owner.process_command.access_mode, target);
    assert_eq!(owner.snapshot.access_mode, target);
    assert!(owner.snapshot.conversation.items.is_empty());

    owner.start_process(None);
    drive_process_until(&mut owner, |owner| {
        owner.startup_state_loaded && owner.startup_history_loaded
    });
    assert!(
        owner.startup_state_loaded && owner.startup_history_loaded,
        "{:?}",
        owner.snapshot.conversation.diagnostics
    );
    assert_eq!(owner.snapshot.access_mode, target);
    Ok(())
}

#[test]
fn app_proxy_change_waits_for_a_running_turn() {
    let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
    conversation_mut(owner.active_snapshot_mut()).running = true;
    let proxy = Some("http://proxy.example:8080".to_owned());

    owner.apply_command(RuntimeCommand::SetAppProxy(proxy.clone()));
    assert_eq!(owner.process_command.app_proxy, proxy);
    assert!(!owner.access_mode_changes.is_idle());

    conversation_mut(owner.active_snapshot_mut()).running = false;
    owner.apply_queued_access_mode_change();
    assert!(owner.access_mode_changes.is_idle());
}

#[test]
fn access_mode_changes_during_a_response_keep_latest_and_allow_cancel() {
    let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
    conversation_mut(owner.active_snapshot_mut()).running = true;
    let full = HarnessAccessMode::Full;

    owner.apply_command(RuntimeCommand::SetAccessMode(full));
    assert_eq!(owner.snapshot.access_mode, full);
    owner.apply_command(RuntimeCommand::SetAccessMode(HarnessAccessMode::Sandboxed));
    assert_eq!(owner.snapshot.access_mode, HarnessAccessMode::Sandboxed);
    assert!(owner.access_mode_changes.is_idle());
    assert_eq!(
        owner.process_command.access_mode,
        HarnessAccessMode::Sandboxed
    );
    assert!(owner.snapshot.conversation.items.is_empty());
}

#[test]
fn access_mode_change_restarts_and_resumes_the_idle_session() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;
    let session = temp.path().join("session.jsonl");
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["history-control".into()]);
    owner.start_process(Some(session));
    drive_process_until(&mut owner, |owner| {
        owner.startup_state_loaded && owner.startup_history_loaded
    });
    assert!(owner.startup_state_loaded && owner.startup_history_loaded);
    assert_eq!(
        owner.snapshot.conversation.items[0].text,
        "preserved history"
    );

    let generation = owner.process_generation;
    let transcript = owner.snapshot.conversation.items.clone();
    let target = HarnessAccessMode::Full;
    owner.apply_command(RuntimeCommand::SetAccessMode(target));

    assert_eq!(owner.process_generation, generation);
    assert_eq!(
        owner.process_command.access_mode,
        HarnessAccessMode::Sandboxed
    );
    assert_eq!(owner.snapshot.access_mode, target);
    assert!(!owner.access_mode_changes.is_idle());

    owner.access_mode_changes.make_due();
    owner.apply_queued_access_mode_change();
    assert_eq!(owner.snapshot.conversation.items, transcript);
    drive_process_until(&mut owner, |owner| {
        owner.startup_state_loaded && owner.startup_history_loaded
    });

    assert!(owner.access_mode_changes.is_idle());
    assert_eq!(owner.process_generation, generation + 1);
    assert!(owner.process.is_some());
    assert_eq!(owner.process_command.access_mode, target);
    assert_eq!(owner.snapshot.access_mode, target);
    assert_eq!(owner.snapshot.conversation.items, transcript);
    Ok(())
}

#[test]
fn history_model_identity_survives_an_unavailable_catalog_entry() {
    let identity = ("opencode-go".into(), "kimi-k3".into());

    assert_eq!(
        HarnessConfigurationStore::history_model(&[], Some(&identity)),
        Some(Model {
            id: "kimi-k3".into(),
            name: "kimi-k3".into(),
            provider: "opencode-go".into(),
            context_window: 0,
            reasoning: false,
            efforts: None,
        })
    );
}

#[test]
fn persisted_submitted_draft_selects_its_session() {
    let project = PathBuf::from("/project");
    let session = PathBuf::from("/sessions/submitted.jsonl");
    assert!(matches!(
        initial_draft_command("draft".into(), project.clone(), Some(session.clone())),
        RuntimeCommand::SelectSession { path, harness, session_id, project: selected_project }
            if path == session
                && harness == "pi"
                && session_id == "/sessions/submitted.jsonl"
                && selected_project == project
    ));
    let codex = PathBuf::from("/locators/codex-cli/thread-1");
    assert!(matches!(
        initial_draft_command("draft".into(), project.clone(), Some(codex.clone())),
        RuntimeCommand::SelectSession { path, harness, session_id, project: selected_project }
            if path == codex
                && harness == "codex-cli"
                && session_id == "thread-1"
                && selected_project == project
    ));
    assert!(matches!(
        initial_draft_command("draft".into(), project.clone(), None),
        RuntimeCommand::ResumeDraft { id, project: draft_project, .. }
            if id == "draft" && draft_project == project
    ));
}

#[test]
fn snapshot_event_clones_share_transcript_storage() {
    let event = RuntimeEvent::Snapshot {
        generation: 1,
        snapshot: Arc::new(RuntimeSnapshot::default()),
    };
    let cloned = event.clone();
    let RuntimeEvent::Snapshot { snapshot: left, .. } = &event else {
        panic!("expected snapshot event");
    };
    let RuntimeEvent::Snapshot {
        snapshot: right, ..
    } = &cloned
    else {
        panic!("expected cloned snapshot event");
    };

    assert!(Arc::ptr_eq(left, right));
}

#[test]
fn cloned_snapshots_share_conversation_storage() {
    let snapshot = RuntimeSnapshot::default();
    let cloned = snapshot.clone();

    assert!(Arc::ptr_eq(&snapshot.conversation, &cloned.conversation));
}

#[test]
fn session_status_publication_deduplicates_but_tracks_session_changes() {
    let (events_tx, events_rx) = mpsc::channel();
    let (wake_tx, _wake_rx) = async_channel::bounded(1);
    let sender = UiEventSender {
        events: events_tx,
        wake: wake_tx,
    };
    let mut published = HashMap::new();

    publish_session_status_if_changed(&sender, &mut published, "target", None, "Working");
    publish_session_status_if_changed(&sender, &mut published, "target", None, "Working");
    publish_session_status_if_changed(
        &sender,
        &mut published,
        "target",
        Some(PathBuf::from("session.jsonl")),
        "Working",
    );

    assert_eq!(events_rx.try_iter().count(), 2);
}

#[test]
fn session_documents_follow_interaction_archive_and_rpc_lifecycle() {
    let mut session = SessionSummary::from_cached(
        "one".into(),
        PathBuf::from("/sessions/one.jsonl"),
        PathBuf::from("/project"),
        "One".into(),
        "Question".into(),
        String::new(),
        None,
        SystemTime::now(),
        2,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );

    assert!(!sessions::document_is_live(&session, false, false));
    assert!(sessions::document_is_live(&session, true, false));
    session.archived = true;
    assert!(!sessions::document_is_live(&session, true, false));
    assert!(sessions::document_is_live(&session, true, true));
    session.is_running = true;
    assert!(sessions::document_is_live(&session, false, false));
}

#[test]
fn running_stats_keep_the_last_meaningful_context_instead_of_flashing_zero() {
    let previous = json!({
        "contextUsage": {"tokens": 168_000, "contextWindow": 200_000, "percent": 84.0},
        "cost": 4
    });
    let next = json!({
        "contextUsage": {"tokens": 0, "contextWindow": 200_000, "percent": 0.0},
        "cost": 5
    });

    let merged = stable_session_stats(&previous, next.clone(), true);
    assert_eq!(merged["contextUsage"], previous["contextUsage"]);
    assert_eq!(merged["cost"], 5);
    assert_eq!(stable_session_stats(&previous, next.clone(), false), next);
}

#[test]
fn first_running_turn_hides_empty_context_until_real_usage_arrives() {
    let next = json!({
        "contextUsage": {"tokens": 0, "contextWindow": 200_000, "percent": 0.0},
        "cost": 0
    });

    let merged = stable_session_stats(&Value::Null, next, true);
    assert!(merged.get("contextUsage").is_none());
}

#[test]
fn failed_tool_does_not_mark_the_whole_session_failed() {
    let mut conversation = ConversationState::default();
    conversation.replace_history(&[
        json!({"role":"assistant","content":[{
            "type":"toolCall","id":"read-1","name":"read","arguments":{"path":"x"}
        }]}),
        json!({
            "role":"toolResult","toolCallId":"read-1","toolName":"read",
            "content":[{"type":"text","text":"not found"}],"isError":true
        }),
    ]);
    assert!(conversation.items[0].is_error);
    assert_eq!(conversation.items[0].kind, TranscriptKind::Tool);
    assert_eq!(
        semantic_status(&RuntimeSnapshot {
            conversation: Arc::new(conversation),
            ..RuntimeSnapshot::default()
        }),
        "Done"
    );
}

#[test]
fn history_preview_does_not_claim_to_know_an_external_run_failed() {
    let mut conversation = ConversationState::default();
    conversation.push_local_error("Previous error", "stale".into());
    assert_eq!(
        semantic_status(&RuntimeSnapshot {
            selected_session: Some(PathBuf::from("session.jsonl")),
            conversation: Arc::new(conversation),
            history_preview: true,
            ..RuntimeSnapshot::default()
        }),
        "Done"
    );
}

#[test]
fn history_uses_latest_assistant_usage_for_context() {
    let messages = vec![json!({
        "role": "assistant",
        "provider": "test",
        "model": "model",
        "usage": {"input": 50, "output": 10, "cacheRead": 20, "cacheWrite": 20, "totalTokens": 100},
        "stopReason": "stop"
    })];
    let models = vec![Model {
        id: "model".into(),
        name: "Model".into(),
        provider: "test".into(),
        context_window: 200,
        reasoning: false,
        efforts: None,
    }];

    let stats = historical_context_stats(&messages, &models);
    assert_eq!(stats["contextUsage"]["tokens"], 100);
    assert_eq!(stats["contextUsage"]["contextWindow"], 200);
    assert_eq!(stats["contextUsage"]["percent"], 50.0);
}

#[test]
fn streaming_usage_updates_context_before_agent_settles() {
    let mut stats = json!({
        "contextUsage": {"tokens": 40, "contextWindow": 200, "percent": 20.0}
    });
    assert!(update_context_from_event(
        &mut stats,
        &json!({
            "type": "message_update",
            "usage": {"input": 60, "output": 10, "cacheRead": 20, "cacheWrite": 10, "totalTokens": 100}
        }),
    ));
    assert_eq!(stats["contextUsage"]["tokens"], 100);
    assert_eq!(stats["contextUsage"]["percent"], 50.0);
    assert!(!update_context_from_event(
        &mut stats,
        &json!({
            "type": "message_update",
            "usage": {"totalTokens": 100}
        }),
    ));
}

#[test]
fn completed_message_usage_updates_context_when_streaming_usage_is_unavailable() {
    let mut stats = json!({
        "contextUsage": {"tokens": 40, "contextWindow": 200, "percent": 20.0}
    });
    update_context_from_event(
        &mut stats,
        &json!({
            "type": "message_end",
            "message": {"usage": {"input": 50, "output": 10, "cacheRead": 20, "cacheWrite": 10}}
        }),
    );
    assert_eq!(stats["contextUsage"]["tokens"], 90);
    assert_eq!(stats["contextUsage"]["percent"], 45.0);
}

#[test]
fn no_op_working_events_do_not_request_a_snapshot_publish() {
    let project = PathBuf::from("/project");
    let (mut owner, events, _discovery) = owner_without_process(project);
    owner.snapshot.status = "Working".into();
    conversation_mut(&mut owner.snapshot).running = true;

    let changed =
        owner.apply_process_item(SessionEvent::Activity(json!({"type": "turn_start"}).into()));

    assert_eq!(changed, SnapshotChange::None);
    assert!(events.try_recv().is_err());
}

#[test]
fn first_agent_action_refreshes_catalog_for_draft_promotion() {
    let (mut owner, events, _discovery) = owner_without_process(PathBuf::from("/project"));
    owner.owns_session_catalog = false;
    owner.active_session = Some(PathBuf::from("/sessions/new.jsonl"));

    owner.apply_process_item(SessionEvent::Activity(json!({"type":"agent_start"}).into()));

    assert!(matches!(
        events.try_recv(),
        Ok(RuntimeEvent::RefreshCatalog)
    ));
}

#[test]
fn agent_end_does_not_report_ready_until_settled() {
    let mut conversation = ConversationState::default();
    conversation.reduce(&json!({"type":"agent_start"}));
    conversation.reduce(&json!({"type":"agent_end","willRetry":false}));
    assert_eq!(run_status(&conversation), "Working");
    conversation.reduce(&json!({"type":"agent_settled"}));
    assert_eq!(run_status(&conversation), "Ready");
}

#[test]
fn notification_target_comes_from_the_emitting_runtime() {
    let snapshot = RuntimeSnapshot {
        project: PathBuf::from("/background-project"),
        live_session: Some(PathBuf::from("/background-session.jsonl")),
        selected_session: Some(PathBuf::from("/history-preview.jsonl")),
        ..RuntimeSnapshot::default()
    };

    assert_eq!(
        notification_target(&snapshot),
        Some((
            PathBuf::from("/background-session.jsonl"),
            PathBuf::from("/background-project"),
        ))
    );
}

#[test]
fn interacted_session_document_hydrates_in_background_and_becomes_resident() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let path = temp.path().join("session.jsonl");
    fs::write(
            &path,
            format!(
                "{{\"type\":\"session\",\"id\":\"one\",\"cwd\":{},\"timestamp\":\"2026-08-19T00:00:00Z\"}}\n{{\"type\":\"message\",\"id\":\"message\",\"parentId\":null,\"message\":{{\"role\":\"user\",\"content\":\"resident\"}}}}\n",
                serde_json::to_string(temp.path()).map_err(|error| error.to_string())?
            ),
        )
        .map_err(|error| error.to_string())?;
    let modified = fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    let session = SessionSummary::from_cached(
        "one".into(),
        path.clone(),
        temp.path().to_path_buf(),
        "One".into(),
        "resident".into(),
        String::new(),
        None,
        modified,
        1,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    let key = format!("session:{}", path.display());
    let mut actors = HashMap::new();
    let mut documents = HashMap::new();
    let mut touches = HashMap::new();
    let mut revisions = HashMap::new();
    let mut paths = HashMap::new();

    reconcile_live_session_documents(
        std::slice::from_ref(&session),
        &HashSet::from([key.clone()]),
        "other",
        &mut actors,
        &mut documents,
        &mut touches,
        &mut revisions,
        &mut paths,
        &AgentLaunchConfig::default(),
        &thread::current(),
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut resident = None;
    while Instant::now() < deadline {
        let Some(actor) = actors.get(&key) else {
            return Err("document actor was not retained".into());
        };
        while let Ok(event) = actor.events.try_recv() {
            if let RuntimeEvent::Snapshot { snapshot, .. } = event
                && snapshot.selected_session.as_deref() == Some(path.as_path())
                && snapshot
                    .conversation
                    .items
                    .iter()
                    .any(|item| item.text == "resident")
            {
                resident = Some(snapshot);
                break;
            }
        }
        if resident.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let resident = resident.ok_or_else(|| "document did not hydrate".to_owned())?;
    documents.insert(key.clone(), resident);
    let mut archived = session;
    archived.archived = true;
    reconcile_live_session_documents(
        &[archived],
        &HashSet::from([key.clone()]),
        "other",
        &mut actors,
        &mut documents,
        &mut touches,
        &mut revisions,
        &mut paths,
        &AgentLaunchConfig::default(),
        &thread::current(),
    );

    assert!(!actors.contains_key(&key));
    assert!(!documents.contains_key(&key));
    assert!(!paths.contains_key(&path));
    assert!(!revisions.contains_key(&path));
    Ok(())
}

#[test]
fn selecting_a_resident_document_does_not_reload_or_message_its_actor() {
    let history = RuntimeSnapshot {
        connected: false,
        history_preview: true,
        ..RuntimeSnapshot::default()
    };
    let disconnected = RuntimeSnapshot::default();

    assert!(!target_command_needs_actor_message(true, Some(&history)));
    assert!(target_command_needs_actor_message(
        true,
        Some(&disconnected)
    ));
    assert!(target_command_needs_actor_message(true, None));
    assert!(target_command_needs_actor_message(false, Some(&history)));
}

#[test]
fn saved_path_reuses_the_actor_that_started_as_a_draft() {
    let path = PathBuf::from("/sessions/one.jsonl");
    let command = RuntimeCommand::SelectSession {
        path: path.clone(),
        harness: "pi".into(),
        session_id: path.to_string_lossy().into_owned(),
        project: PathBuf::from("/project"),
    };
    let latest = HashMap::from([(
        "draft:one".into(),
        Arc::new(RuntimeSnapshot {
            live_session: Some(path.clone()),
            ..RuntimeSnapshot::default()
        }),
    )]);

    assert!(is_view_only_selection(&command));
    assert_eq!(
        actor_key_for_command(&command, &format!("session:{}", path.display()), &latest,),
        "draft:one"
    );

    let restart = RuntimeCommand::RestartSession {
        path: path.clone(),
        harness: "pi".into(),
        session_id: path.to_string_lossy().into_owned(),
        project: PathBuf::from("/project"),
    };
    assert!(!is_view_only_selection(&restart));
    assert_eq!(
        actor_key_for_command(&restart, &format!("session:{}", path.display()), &latest,),
        "draft:one"
    );
}

#[test]
fn non_catalog_discovery_refreshes_catalog_without_publishing_local_results() {
    let action = route_session_discovery(
        "draft:a",
        "catalog",
        RuntimeEvent::Sessions {
            generation: 99,
            sessions: Vec::new(),
            all_sessions: Vec::new(),
            activities: None,
        },
    );

    assert!(matches!(action, SupervisorSessionAction::RefreshCatalog));
}

#[test]
fn catalog_discovery_is_the_authoritative_generation_namespace() {
    let action = route_session_discovery(
        "catalog",
        "catalog",
        RuntimeEvent::Sessions {
            generation: 7,
            sessions: Vec::new(),
            all_sessions: Vec::new(),
            activities: None,
        },
    );

    assert!(matches!(
        action,
        SupervisorSessionAction::Publish(RuntimeEvent::Sessions { generation: 7, .. })
    ));
}

#[test]
fn non_catalog_discovery_failures_also_refresh_instead_of_publishing() {
    let action = route_session_discovery(
        "session:/one",
        "catalog",
        RuntimeEvent::SessionsFailed {
            generation: 41,
            message: "local failure".into(),
        },
    );

    assert!(matches!(action, SupervisorSessionAction::RefreshCatalog));
}

#[test]
fn non_catalog_actors_request_one_authoritative_refresh_without_scanning() {
    let (mut owner, events, _discovery) = owner_without_process(std::env::temp_dir());
    owner.owns_session_catalog = false;

    owner.refresh_sessions();

    assert_eq!(owner.session_generation, 0);
    assert!(matches!(
        events.try_recv(),
        Ok(RuntimeEvent::RefreshCatalog)
    ));
}

#[test]
fn streaming_accepts_queued_messages_and_exact_extension_commands() {
    assert!(!can_send_prompt(PromptMode::Normal, true, false));
    assert!(can_send_prompt(PromptMode::Steer, true, false));
    assert!(can_send_prompt(PromptMode::FollowUp, true, false));
    assert!(can_send_prompt(PromptMode::Normal, false, false));
    assert!(can_send_prompt(PromptMode::Normal, true, true));
}

#[test]
fn reload_is_rejected_while_the_session_is_running() {
    let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
    conversation_mut(&mut owner.snapshot).reduce(&json!({"type":"agent_start"}));
    let generation = owner.process_generation;

    owner.apply_command(RuntimeCommand::Reload);

    assert_eq!(owner.process_generation, generation);
    assert_eq!(owner.snapshot.status, "Reload not started");
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.text.contains("Wait for the current response"))
    );
}

#[test]
fn reload_restarts_the_idle_session_process() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))?;
    let session = temp.path().join("session.jsonl");
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["quiet".into()]);
    owner.active_session = Some(session.clone());
    owner.snapshot.selected_session = Some(session.clone());
    let generation = owner.process_generation;

    owner.apply_command(RuntimeCommand::Reload);

    assert!(owner.process.is_some());
    assert_eq!(owner.process_generation, generation + 1);
    assert_eq!(owner.snapshot.selected_session, Some(session));
    assert!(owner.snapshot.connected);
    if let Some(mut process) = owner.process.take() {
        process.close()?;
    }
    Ok(())
}

#[test]
fn initial_prompt_is_not_duplicated_when_starting_its_process()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))?;
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["quiet".into()]);
    owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);

    owner.send_prompt(
        "draft:a".into(),
        PromptMode::Normal,
        "hello".into(),
        Vec::new(),
        false,
    );

    assert!(owner.deferred_prompt.is_some());
    assert_eq!(owner.snapshot.conversation.items.len(), 1);
    owner.active_session = Some(temp.path().join("session.jsonl"));
    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.maybe_send_deferred_prompt();

    let user_messages = owner
        .snapshot
        .conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User && item.text == "hello")
        .count();
    assert_eq!(user_messages, 1);
    if let Some(mut process) = owner.process.take() {
        process.close()?;
    }
    Ok(())
}

#[test]
fn deferred_prompt_is_rejected_when_startup_state_has_no_session_path()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))?;
    let process = crate::agents::spawn_session(
        &AgentLaunchConfig::test_script(&script, vec!["quiet".into()]),
        SessionLaunch {
            harness: "pi".into(),
            session_id: None,
            project: temp.path().to_path_buf(),
            start: SessionStart::New,
            wake: None,
        },
    )?;
    let (mut owner, events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process = Some(process);
    owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);

    owner.send_prompt(
        "draft:a".into(),
        PromptMode::Normal,
        "hello".into(),
        Vec::new(),
        false,
    );

    assert!(owner.deferred_prompt.is_some());
    assert!(
        events
            .try_iter()
            .all(|event| !matches!(event, RuntimeEvent::PromptResult { .. }))
    );

    owner.startup_state_loaded = true;
    owner.startup_history_loaded = true;
    owner.maybe_send_deferred_prompt();

    assert!(owner.deferred_prompt.is_none());
    assert!(owner.pending_prompt_item.is_none());
    assert!(!owner.snapshot.conversation.running);
    assert!(
        owner
            .state
            .as_ref()
            .expect("state")
            .queued_prompts()?
            .is_empty()
    );
    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            target,
            accepted: false,
            session: None,
            ..
        } if target == "draft:a"
    )));
    Ok(())
}

#[test]
fn accepted_prompt_result_has_the_normalized_active_session_path()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let session = temp.path().join("session.jsonl");
    let link = temp.path().join("link.jsonl");
    fs::write(&session, "{}")?;
    symlink(&session, &link)?;
    let (mut owner, events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.active_session = Some(crate::sessions::normalize_session_path(&link));

    owner.emit_prompt_result("draft:a", true);

    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            accepted: true,
            session: Some(path),
            ..
        } if path == session.canonicalize().expect("canonical session")
    )));
    Ok(())
}

#[test]
fn new_session_stays_cold_until_the_first_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let old_project = tempdir()?;
    let new_project = tempdir()?;
    let script = old_project.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))?;
    let (mut owner, _events, _discovery) = owner_without_process(old_project.path().to_path_buf());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["quiet".into()]);
    owner.state = Some(StateStore::open_at(
        &old_project.path().join("state.sqlite3"),
    )?);

    owner.apply_command(RuntimeCommand::NewSession {
        id: "draft-new".into(),
        harness: "pi".into(),
        project: new_project.path().to_path_buf(),
    });

    assert_eq!(owner.project, new_project.path());
    assert!(owner.process.is_none());
    assert!(!owner.snapshot.connected);

    owner.send_prompt(
        "draft:draft-new".into(),
        PromptMode::Normal,
        "start".into(),
        Vec::new(),
        false,
    );

    assert!(owner.process.is_some());
    assert!(owner.deferred_prompt.is_some());
    assert!(owner.snapshot.connected);
    if let Some(mut process) = owner.process.take() {
        process.close()?;
    }
    Ok(())
}

#[test]
fn cold_draft_model_selection_is_deferred_without_starting_the_harness() {
    let project = std::env::temp_dir().join("cold-model-project");
    let (mut owner, _events, _discovery) = owner_without_process(project.clone());
    let model = Model {
        id: "model".into(),
        name: "Model".into(),
        provider: "provider".into(),
        context_window: 1,
        reasoning: true,
        efforts: None,
    };
    owner.apply_command(RuntimeCommand::NewSession {
        id: "draft-cold".into(),
        harness: "pi".into(),
        project,
    });

    owner.apply_command(RuntimeCommand::SetModel(model.clone()));

    assert!(owner.process.is_none());
    assert!(!owner.pending_session_controls.is_empty());
    assert_eq!(owner.snapshot.prefill_model, Some(model));
}

#[test]
fn cold_model_selection_replaces_an_unsupported_effort() {
    let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
    owner.snapshot.prefill_thinking_level = Some("high".into());

    owner.apply_command(RuntimeCommand::SetModel(Model {
        id: "limited".into(),
        name: "Limited".into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning: true,
        efforts: Some(vec!["low".into(), "medium".into()]),
    }));

    assert_eq!(
        owner.snapshot.prefill_thinking_level.as_deref(),
        Some("medium")
    );
    assert!(!owner.pending_session_controls.is_empty());
}

#[test]
fn background_catalog_refresh_preserves_search_until_user_clears_it()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let alpha_path = temp.path().join("alpha.jsonl");
    let beta_path = temp.path().join("beta.jsonl");
    fs::write(&alpha_path, "{}")?;
    fs::write(&beta_path, "{}")?;
    let summary = |id: &str, path: PathBuf, search: &str| {
        SessionSummary::from_cached(
            id.into(),
            path,
            project.clone(),
            id.into(),
            String::new(),
            String::new(),
            None,
            SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            false,
            false,
            search.into(),
        )
    };
    let sessions = vec![
        summary("alpha", alpha_path.canonicalize()?, "alpha"),
        summary("beta", beta_path.canonicalize()?, "beta"),
    ];
    let (mut owner, events, _discovery) = owner_without_process(project);
    owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);
    owner
        .state
        .as_mut()
        .expect("state")
        .replace_sessions(&sessions)?;

    owner.load_sessions("alpha".into());
    let searched = events.try_iter().collect::<Vec<_>>();
    assert!(searched.iter().any(|event| matches!(
        event,
        RuntimeEvent::Sessions { sessions, .. }
            if sessions.len() == 1 && sessions[0].id == "alpha"
    )));

    owner.refresh_sessions();
    let refresh_generation = owner.session_generation;
    assert_eq!(owner.session_query, "alpha");
    owner.apply_discovery(DiscoveryResult {
        generation: refresh_generation,
        result: Ok(SessionDiscovery {
            sessions,
            activities: HashMap::new(),
            exhaustive: true,
        }),
    });
    let refreshed = events.try_iter().collect::<Vec<_>>();
    assert!(refreshed.iter().any(|event| matches!(
        event,
        RuntimeEvent::Sessions { sessions, .. }
            if sessions.len() == 1 && sessions[0].id == "alpha"
    )));
    assert_eq!(owner.session_query, "alpha");

    owner.load_sessions(String::new());
    let cleared = events.try_iter().collect::<Vec<_>>();
    assert!(cleared.iter().any(|event| matches!(
        event,
        RuntimeEvent::Sessions { sessions, .. } if sessions.len() == 2
    )));
    assert!(owner.session_query.is_empty());
    Ok(())
}

#[test]
fn filesystem_refresh_commands_coalesce_into_one_delayed_scan() {
    let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
    let command = RuntimeCommand::ScheduleSessionRefresh;
    assert!(command_targets_catalog(&command));
    owner.apply_command(command.clone());
    let due = owner.session_refresh_due.expect("filesystem refresh");
    owner.apply_command(command);
    assert_eq!(owner.session_refresh_due, Some(due));

    owner.poll_deferred_session_refresh(due - Duration::from_millis(1));
    assert_eq!(owner.session_generation, 0);
    owner.poll_deferred_session_refresh(due);
    assert_eq!(owner.session_generation, 1);
    assert!(owner.session_discovery_in_flight);
}

#[test]
fn worker_start_tools_request_catalog_refresh() {
    for name in ["collaboration.spawn_agent", "spawnAgent", "worker_start"] {
        assert!(tool_starts_worker(
            &SessionActivityKind::ToolStarted,
            &json!({"toolName": name, "args": {}}),
        ));
    }
    assert!(tool_starts_worker(
        &SessionActivityKind::ToolStarted,
        &json!({"toolName": "mcp__farcaster__worker_send", "args": {"to": "diff-review"}}),
    ));
    assert!(tool_starts_worker(
        &SessionActivityKind::ToolStarted,
        &json!({"toolName": "mcp__farcaster__worker_send", "args": {"to": "ignored-by-child"}}),
    ));
    assert!(!tool_starts_worker(
        &SessionActivityKind::ToolStarted,
        &json!({"toolName": "read", "args": {}}),
    ));
    assert!(!tool_starts_worker(
        &SessionActivityKind::ToolStarted,
        &json!({"toolName": "todoist.create_task", "args": {}}),
    ));
}

#[test]
fn in_flight_catalog_refreshes_coalesce_into_one_delayed_scan() {
    let (mut owner, _events, _discovery) = owner_without_process(std::env::temp_dir());
    owner.session_discovery_in_flight = true;
    owner.refresh_sessions();
    owner.refresh_sessions();
    assert!(owner.session_refresh_pending);

    owner.apply_discovery(DiscoveryResult {
        generation: 0,
        result: Ok(SessionDiscovery {
            sessions: Vec::new(),
            activities: HashMap::new(),
            exhaustive: true,
        }),
    });
    let due = owner.session_refresh_due.expect("deferred refresh");
    assert!(!owner.session_refresh_pending);
    owner.refresh_sessions();
    assert_eq!(owner.session_generation, 0);
    owner.poll_deferred_session_refresh(due - Duration::from_millis(1));
    assert_eq!(owner.session_generation, 0);

    owner.poll_deferred_session_refresh(due);
    assert_eq!(owner.session_generation, 1);
    assert!(owner.session_discovery_in_flight);
    owner.poll_deferred_session_refresh(due + Duration::from_secs(1));
    assert_eq!(owner.session_generation, 1);
}

#[test]
fn cached_child_only_search_publishes_tree_closure_and_unfiltered_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let root_path = temp.path().join("root.jsonl");
    let child_path = temp.path().join("child.jsonl");
    fs::write(&root_path, "{}")?;
    fs::write(&child_path, "{}")?;
    let root = SessionSummary::from_cached(
        "root".into(),
        root_path.canonicalize()?,
        project.clone(),
        "Main".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        "ordinary".into(),
    );
    let child = SessionSummary::from_cached(
        "child".into(),
        child_path.canonicalize()?,
        project.clone(),
        "Implementation child".into(),
        "Needle assignment".into(),
        String::new(),
        Some("root".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        true,
        "needle assignment".into(),
    );
    let (mut owner, events, _) = owner_without_process(project);
    owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);
    owner
        .state
        .as_mut()
        .expect("state")
        .replace_sessions(&[root, child])?;

    owner.load_sessions("needle".into());

    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::Sessions { sessions, all_sessions, .. }
            if sessions.iter().map(|session| session.id.as_str()).collect::<HashSet<_>>()
                == HashSet::from(["root", "child"])
                && all_sessions.len() == 2
    )));
    Ok(())
}

#[test]
fn partial_catalog_refresh_keeps_omitted_running_children_in_the_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let root_path = temp.path().join("root.jsonl");
    let child_path = temp.path().join("child.jsonl");
    fs::write(&root_path, "{}")?;
    fs::write(&child_path, "{}")?;
    let summary = |id: &str, path: PathBuf, parent: Option<&str>, is_running: bool| {
        SessionSummary::from_cached(
            id.into(),
            path,
            project.clone(),
            id.into(),
            String::new(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            false,
            is_running,
            id.into(),
        )
    };
    let root = summary("root", root_path.canonicalize()?, None, false);
    let child = summary("child", child_path.canonicalize()?, Some("root"), true);
    let (mut owner, events, _discovery) = owner_without_process(project);
    owner.state = Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?);
    owner
        .state
        .as_mut()
        .expect("state")
        .replace_sessions(&[root.clone(), child])?;

    owner.apply_discovery(DiscoveryResult {
        generation: 0,
        result: Ok(SessionDiscovery {
            sessions: vec![root],
            activities: HashMap::new(),
            exhaustive: false,
        }),
    });

    assert!(events.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::Sessions { all_sessions, .. }
            if all_sessions.iter().any(|session| session.id == "child")
    )));
    Ok(())
}

#[test]
fn first_session_path_triggers_a_sidebar_refresh() {
    let project = std::env::temp_dir();
    let session = project.join("new-session.jsonl");
    let (mut owner, _events, _discovery) = owner_without_process(project);

    owner.apply_response(crate::agents::SessionResponse {
        id: Some("state".into()),
        operation: crate::agents::SessionOperation::LoadState,
        success: true,
        data: json!({
            "model": null,
            "thinkingLevel": "off",
            "isStreaming": true,
            "isCompacting": false,
            "sessionFile": session,
            "sessionId": "new-session",
            "sessionName": null,
            "autoCompactionEnabled": true,
            "messageCount": 1,
            "pendingMessageCount": 0
        }),
        error: None,
    });

    assert_eq!(owner.active_session, Some(session));
    assert_eq!(owner.session_generation, 1);
}

#[test]
fn get_state_canonicalizes_a_symlinked_session_path() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let session = temp.path().join("session.jsonl");
    let link = temp.path().join("linked.jsonl");
    fs::write(&session, "{}")?;
    symlink(&session, &link)?;
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());

    owner.apply_response(crate::agents::SessionResponse {
        id: Some("state".into()),
        operation: crate::agents::SessionOperation::LoadState,
        success: true,
        data: json!({
            "model": null,
            "thinkingLevel": "off",
            "isStreaming": false,
            "isCompacting": false,
            "sessionFile": link,
            "sessionId": "session",
            "sessionName": null,
            "autoCompactionEnabled": true,
            "messageCount": 0,
            "pendingMessageCount": 0
        }),
        error: None,
    });

    assert_eq!(owner.active_session, Some(session.canonicalize()?));
    assert_eq!(
        owner.snapshot.selected_session,
        Some(session.canonicalize()?)
    );
    Ok(())
}

#[test]
fn starting_session_prefills_controls_from_the_last_ready_session() {
    let model = Model {
        id: "model-1".into(),
        name: "Model One".into(),
        provider: "provider-1".into(),
        context_window: 200_000,
        reasoning: true,
        efforts: None,
    };
    let mut controls = HarnessConfigurationStore::default();
    let mut ready = RuntimeSnapshot {
        session: serde_json::from_value(json!({
            "model": {
                "id": "model-1",
                "name": "Model One",
                "provider": "provider-1",
                "reasoning": true
            },
            "thinkingLevel": "high",
            "isStreaming": false,
            "isCompacting": false,
            "sessionFile": "/old",
            "sessionId": "old",
            "autoCompactionEnabled": true,
            "messageCount": 0,
            "pendingMessageCount": 0
        }))
        .ok(),
        models: vec![model.clone()],
        thinking_levels: vec!["off".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };
    controls.reconcile_snapshot(&mut ready, true);

    let mut starting = RuntimeSnapshot::default();
    controls.reconcile_snapshot(&mut starting, true);

    assert_eq!(starting.prefill_model, Some(model.clone()));
    assert_eq!(starting.prefill_thinking_level.as_deref(), Some("high"));
    assert_eq!(starting.models, vec![model]);
    assert_eq!(starting.thinking_levels, vec!["off", "high"]);
    assert!(starting.session.is_none());
}

#[test]
fn history_identity_overrides_draft_defaults_without_changing_them() {
    let sol = Model {
        id: "gpt-5.6-sol".into(),
        name: "Sol".into(),
        provider: "openai-codex".into(),
        context_window: 200_000,
        reasoning: true,
        efforts: None,
    };
    let luna = Model {
        id: "gpt-5.6-luna".into(),
        name: "Luna".into(),
        provider: "openai-codex".into(),
        context_window: 200_000,
        reasoning: true,
        efforts: None,
    };
    let mut defaults = HarnessConfigurationStore::default();
    let mut live_sol = RuntimeSnapshot {
        session: serde_json::from_value(json!({
            "model": sol,
            "thinkingLevel": "high",
            "isStreaming": false,
            "isCompacting": false,
            "sessionFile": "/sol",
            "sessionId": "sol",
            "autoCompactionEnabled": true,
            "messageCount": 0,
            "pendingMessageCount": 0
        }))
        .ok(),
        models: vec![sol.clone(), luna.clone()],
        thinking_levels: vec!["medium".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut live_sol, true);

    let mut luna_history = RuntimeSnapshot {
        prefill_model: Some(luna.clone()),
        prefill_thinking_level: Some("medium".into()),
        history_preview: true,
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut luna_history, false);

    let history_identity = luna_history.session_identity();
    assert_eq!(history_identity.provider, Some("openai-codex"));
    assert_eq!(history_identity.model, Some(&luna));
    assert_eq!(history_identity.effort, Some("medium"));

    let mut empty_draft = RuntimeSnapshot::default();
    defaults.reconcile_snapshot(&mut empty_draft, true);
    let draft_identity = empty_draft.session_identity();
    assert_eq!(draft_identity.model, Some(&sol));
    assert_eq!(draft_identity.effort, Some("high"));
}

#[test]
fn viewing_a_subagent_does_not_change_new_session_defaults() {
    let sol = Model {
        id: "gpt-5.6-sol".into(),
        name: "Sol".into(),
        provider: "openai-codex".into(),
        context_window: 200_000,
        reasoning: true,
        efforts: None,
    };
    let luna = Model {
        id: "gpt-5.6-luna".into(),
        name: "Luna".into(),
        provider: "openai-codex".into(),
        context_window: 200_000,
        reasoning: true,
        efforts: None,
    };
    let session_state = |path: &str, model: Model| {
        serde_json::from_value(json!({
            "model": model,
            "thinkingLevel": "high",
            "isStreaming": false,
            "isCompacting": false,
            "sessionFile": path,
            "sessionId": path,
            "autoCompactionEnabled": true,
            "messageCount": 0,
            "pendingMessageCount": 0
        }))
        .ok()
    };
    let mut defaults = HarnessConfigurationStore::default();
    let mut root = RuntimeSnapshot {
        session: session_state("/sessions/root.jsonl", sol.clone()),
        models: vec![sol.clone(), luna.clone()],
        thinking_levels: vec!["medium".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut root, true);

    // A descendant's model must not replace the defaults inherited by new drafts.
    let mut subagent = RuntimeSnapshot {
        live_session: Some(PathBuf::from("/sessions/child.jsonl")),
        session: session_state("/sessions/child.jsonl", luna.clone()),
        models: vec![sol.clone(), luna.clone()],
        thinking_levels: vec!["medium".into(), "high".into()],
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut subagent, false);

    let mut new_draft = RuntimeSnapshot::default();
    defaults.reconcile_snapshot(&mut new_draft, true);
    let identity = new_draft.session_identity();
    assert_eq!(identity.model, Some(&sol));
    assert_eq!(identity.effort, Some("high"));
}

#[test]
fn cold_drafts_reuse_only_their_own_harness_catalog() {
    let pi_model = Model {
        id: "pi-model".into(),
        name: "Pi Model".into(),
        provider: "pi-provider".into(),
        context_window: 1,
        reasoning: false,
        efforts: None,
    };
    let mut defaults = HarnessConfigurationStore::default();
    let mut pi = RuntimeSnapshot {
        harness: "pi".into(),
        models: vec![pi_model.clone()],
        thinking_levels: vec!["high".into()],
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut pi, true);

    let mut codex = RuntimeSnapshot {
        harness: "codex-cli".into(),
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut codex, true);
    assert!(codex.models.is_empty());
    assert!(codex.thinking_levels.is_empty());

    let mut next_pi = RuntimeSnapshot {
        harness: "pi".into(),
        ..RuntimeSnapshot::default()
    };
    defaults.reconcile_snapshot(&mut next_pi, true);
    assert_eq!(next_pi.models, vec![pi_model]);
    assert_eq!(next_pi.thinking_levels, vec!["high"]);
}

#[test]
fn startup_delivers_all_composer_steers_at_one_turn_boundary() {
    assert_eq!(startup_commands()[0], SessionCommand::ConfigureSteering);
}

#[test]
fn process_replacement_clears_all_session_owned_snapshot_state() {
    let mut snapshot = RuntimeSnapshot {
        connected: true,
        status: "old".into(),
        session: serde_json::from_value(json!({
            "model": null,
            "thinkingLevel": "high",
            "isStreaming": true,
            "isCompacting": true,
            "sessionFile": "/old",
            "sessionId": "old",
            "autoCompactionEnabled": true,
            "messageCount": 9,
            "pendingMessageCount": 2
        }))
        .ok(),
        selected_session: Some(PathBuf::from("/old")),
        models: vec![Model {
            id: "old".into(),
            name: "Old".into(),
            provider: "test".into(),
            context_window: 0,
            reasoning: false,
            efforts: None,
        }],
        thinking_levels: vec!["high".into()],
        stats: json!({"tokens": 10}),
        commands: vec![SlashCommand {
            name: "old".into(),
            description: None,
            source: crate::protocol::SlashCommandSource::Extension,
        }],
        stderr: "old stderr".into(),
        auto_retry: false,
        ..RuntimeSnapshot::default()
    };
    conversation_mut(&mut snapshot).reduce(&json!({"type": "agent_start"}));
    conversation_mut(&mut snapshot).reduce(&json!({
        "type": "queue_update",
        "steering": ["old"],
        "followUp": ["later"]
    }));

    reset_snapshot_for_process(
        &mut snapshot,
        PathBuf::from("/new-project"),
        Some(PathBuf::from("/new")),
        "Resuming session".into(),
    );

    assert!(!snapshot.connected);
    assert_eq!(snapshot.status, "Resuming session");
    assert_eq!(snapshot.project, PathBuf::from("/new-project"));
    assert_eq!(snapshot.selected_session, Some(PathBuf::from("/new")));
    assert!(!snapshot.auto_retry);
    assert_eq!(snapshot.session, None);
    assert!(snapshot.models.is_empty());
    assert!(snapshot.thinking_levels.is_empty());
    assert_eq!(snapshot.stats, Value::Null);
    assert!(snapshot.commands.is_empty());
    assert!(snapshot.stderr.is_empty());
    assert_eq!(*snapshot.conversation, ConversationState::default());
}

#[test]
fn model_change_from_history_reconnects_without_hiding_history() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;
    let session = temp.path().join("history.jsonl");
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["history-control".into()]);
    preview_history(&mut owner, session.clone(), "preserved history");

    owner.apply_command(RuntimeCommand::SetModel(Model {
        id: "new-model".into(),
        name: "New Model".into(),
        provider: "new-provider".into(),
        context_window: 0,
        reasoning: true,
        efforts: None,
    }));

    assert!(owner.process.is_some());
    assert!(owner.snapshot.history_preview);
    assert_eq!(
        owner.snapshot.conversation.items[0].text,
        "preserved history"
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        while let Some(item) = owner.process.as_mut().and_then(|process| process.poll()) {
            owner.apply_process_item(item);
        }
        if owner.pending_session_controls.is_empty()
            && owner
                .snapshot
                .session
                .as_ref()
                .and_then(|state| state.model.as_ref())
                .is_some_and(|model| model.id == "new-model")
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(!owner.snapshot.history_preview);
    assert_eq!(owner.snapshot.selected_session, Some(session));
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .any(|item| { item.kind == TranscriptKind::User && item.text == "preserved history" })
    );
    assert_eq!(
        owner
            .snapshot
            .session
            .as_ref()
            .and_then(|state| state.model.as_ref())
            .map(|model| (model.provider.as_str(), model.id.as_str())),
        Some(("new-provider", "new-model"))
    );
    Ok(())
}

#[test]
fn failed_model_reconnect_keeps_the_loaded_history() {
    let project = std::env::temp_dir();
    let session = project.join("history.jsonl");
    let (mut owner, _events, _discovery) = owner_without_process(project);
    owner.process_command = AgentLaunchConfig {
        program: PathBuf::from("/definitely/missing/farcaster-test-command"),
        prefix_args: Vec::new(),
        access_mode: HarnessAccessMode::default(),
        app_proxy: None,
        session_locator_root: None,
    };
    preview_history(&mut owner, session.clone(), "keep this history");

    owner.apply_command(RuntimeCommand::SetModel(Model {
        id: "model".into(),
        name: "Model".into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning: true,
        efforts: None,
    }));

    assert!(owner.process.is_none());
    assert!(owner.snapshot.history_preview);
    assert_eq!(owner.snapshot.selected_session, Some(session));
    assert_eq!(
        owner.snapshot.conversation.items[0].text,
        "keep this history"
    );
    assert!(
        owner.snapshot.conversation.items.iter().any(|item| {
            item.kind == TranscriptKind::Error && item.label == "Couldn’t start Pi"
        })
    );
}

#[test]
fn failed_resume_publishes_no_state_from_the_previous_process() {
    let (event_tx, event_rx) = test_event_channel();
    let (discovery_tx, _discovery_rx) = mpsc::channel();
    let (history_tx, _history_rx) = mpsc::channel();
    let mut owner = RuntimeOwner {
        project: std::env::temp_dir(),
        harness: "pi".into(),
        session_id: None,
        process_command: AgentLaunchConfig {
            program: PathBuf::from("/definitely/missing/farcaster-test-command"),
            prefix_args: Vec::new(),
            access_mode: HarnessAccessMode::default(),
            app_proxy: None,
            session_locator_root: None,
        },
        process: None,
        snapshot: RuntimeSnapshot {
            connected: true,
            status: "old".into(),
            selected_session: Some(PathBuf::from("/old")),
            models: vec![Model {
                id: "old".into(),
                name: "Old".into(),
                provider: "test".into(),
                context_window: 0,
                reasoning: false,
                efforts: None,
            }],
            thinking_levels: vec!["high".into()],
            stats: json!({"old": true}),
            commands: vec![SlashCommand {
                name: "old".into(),
                description: None,
                source: crate::protocol::SlashCommandSource::Extension,
            }],
            stderr: "old stderr".into(),
            auto_retry: true,
            ..RuntimeSnapshot::default()
        },
        owns_session_catalog: false,
        session_generation: 0,
        session_discovery_in_flight: false,
        session_refresh_pending: false,
        session_refresh_due: None,
        process_generation: 4,
        pending_prompt_id: None,
        pending_prompt_target: None,
        pending_prompt_item: None,
        pending_outbox_id: None,
        title_generation: SessionTitleGeneration::default(),
        transcript_changed_from: None,
        event_tx,
        discovery_tx,
        history_tx,
        history_generation: 1,
        history_selection_generation: None,
        document_refresh_generation: Some(1),
        pending_document_refresh: None,
        active_session: Some(PathBuf::from("/old")),
        parked_snapshot: None,
        deferred_prompt: None,
        pending_session_controls: PendingSessionControls::default(),
        access_mode_changes: AccessModeChangeState::default(),
        startup_state_loaded: false,
        startup_history_loaded: false,
        state: None,
        session_query: String::new(),
    };
    conversation_mut(&mut owner.snapshot).reduce(&json!({
        "type": "queue_update",
        "steering": ["old"],
        "followUp": []
    }));

    owner.start_process(Some(PathBuf::from("/new")));

    let snapshots = event_rx
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        })
        .collect::<Vec<_>>();
    let latest = snapshots.last().expect("failed replacement should publish");
    assert_eq!(latest.selected_session, Some(PathBuf::from("/new")));
    assert!(latest.models.is_empty());
    assert!(latest.thinking_levels.is_empty());
    assert_eq!(latest.stats, Value::Null);
    assert!(latest.commands.is_empty());
    assert!(latest.stderr.is_empty());
    assert!(latest.conversation.queue.steering.is_empty());
    let error = latest
        .conversation
        .items
        .iter()
        .find(|item| item.kind == TranscriptKind::Error)
        .expect("failed process should publish a visible error");
    assert_eq!(error.label, "Couldn’t start Pi");
    assert!(error.text.contains("definitely/missing"));
    assert!(error.tool_output.contains("definitely/missing"));
    assert!(
        latest
            .conversation
            .diagnostics
            .iter()
            .any(|item| item.contains("definitely/missing"))
    );

    owner.apply_history(HistoryResult {
        generation: 1,
        path: PathBuf::from("/stale-history"),
        project: std::env::temp_dir(),
        kind: HistoryLoadKind::DocumentRefresh,
        result: Ok(LoadedHistory {
            messages: vec![json!({"role":"user","content":"stale"})],
            model: None,
            thinking_level: None,
            pending_question: None,
        }),
    });
    assert_eq!(owner.snapshot.selected_session, Some(PathBuf::from("/new")));
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .all(|item| item.text != "stale")
    );
}

#[test]
fn failed_start_marks_the_deferred_prompt_failed() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let database = temp.path().join("gui-state.sqlite3");
    let (mut owner, _events, _discovery) = owner_without_process(temp.path().to_path_buf());
    owner.process_command = AgentLaunchConfig {
        program: PathBuf::from("/definitely/missing/farcaster-test-command"),
        prefix_args: Vec::new(),
        access_mode: HarnessAccessMode::default(),
        app_proxy: None,
        session_locator_root: None,
    };
    owner.state = Some(StateStore::open_at(&database)?);

    owner.send_prompt(
        "draft:failed-start".into(),
        PromptMode::Normal,
        "hello".into(),
        Vec::new(),
        false,
    );

    assert!(
        owner
            .state
            .as_ref()
            .expect("state")
            .queued_prompts()?
            .is_empty()
    );
    assert!(owner.pending_outbox_id.is_none());
    let connection = rusqlite::Connection::open(database)?;
    let (state, error) = connection.query_row(
        "SELECT state, error FROM outbox ORDER BY id DESC LIMIT 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    assert_eq!(state, "failed");
    assert!(error.contains("definitely/missing"));
    Ok(())
}

#[test]
fn selecting_from_an_idle_session_does_not_start_pi() {
    let old_project = PathBuf::from("/old-project");
    let new_project = PathBuf::from("/new-project");
    let old_path = PathBuf::from("/old-session.jsonl");
    let new_path = PathBuf::from("/new-session.jsonl");
    let (mut owner, events, _discovery) = owner_without_process(old_project);
    owner.active_session = Some(old_path.clone());
    owner.snapshot.selected_session = Some(old_path.clone());
    owner.process_generation = 4;

    owner.select_history(new_path, new_project);

    assert_eq!(owner.process_generation, 4);
    assert_eq!(owner.active_session, Some(old_path));
    assert!(owner.process.is_none());
    assert!(
        events
            .try_iter()
            .all(|event| !matches!(event, RuntimeEvent::SessionReset { .. }))
    );
}

#[test]
fn history_preview_keeps_running_pi_until_a_prompt_resumes_the_session() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let script = temp.path().join("fake-pi.sh");
    fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;
    let process_command = AgentLaunchConfig::test_script(&script, vec!["quiet".into()]);
    let process = crate::agents::spawn_session(
        &process_command,
        SessionLaunch {
            harness: "pi".into(),
            session_id: None,
            project: temp.path().to_path_buf(),
            start: SessionStart::New,
            wake: None,
        },
    )?;
    let (event_tx, event_rx) = test_event_channel();
    let (discovery_tx, _discovery_rx) = mpsc::channel();
    let (history_tx, _history_rx) = mpsc::channel();
    let old_path = PathBuf::from("/old");
    let new_path = PathBuf::from("/new");
    let old_project = temp.path().to_path_buf();
    let new_project = temp.path().join("other-project");
    fs::create_dir(&new_project).map_err(|error| error.to_string())?;
    let mut owner = RuntimeOwner {
        project: old_project.clone(),
        harness: "pi".into(),
        session_id: None,
        process_command,
        process: Some(process),
        snapshot: RuntimeSnapshot {
            connected: true,
            status: "Working".into(),
            project: old_project.clone(),
            selected_session: Some(old_path.clone()),
            ..RuntimeSnapshot::default()
        },
        owns_session_catalog: false,
        session_generation: 0,
        session_discovery_in_flight: false,
        session_refresh_pending: false,
        session_refresh_due: None,
        process_generation: 3,
        pending_prompt_id: None,
        pending_prompt_target: None,
        pending_prompt_item: None,
        pending_outbox_id: None,
        title_generation: SessionTitleGeneration::default(),
        transcript_changed_from: None,
        event_tx,
        discovery_tx,
        history_tx,
        history_generation: 1,
        history_selection_generation: None,
        document_refresh_generation: None,
        pending_document_refresh: None,
        active_session: Some(old_path.clone()),
        parked_snapshot: None,
        deferred_prompt: None,
        pending_session_controls: PendingSessionControls::default(),
        access_mode_changes: AccessModeChangeState::default(),
        startup_state_loaded: false,
        startup_history_loaded: false,
        state: Some(StateStore::open_at(&temp.path().join("gui-state.sqlite3"))?),
        session_query: String::new(),
    };
    conversation_mut(&mut owner.snapshot).running = true;

    owner.select_history(old_path.clone(), old_project);
    owner.apply_history(HistoryResult {
        generation: 1,
        path: new_path.clone(),
        project: new_project.clone(),
        kind: HistoryLoadKind::Selection,
        result: Ok(crate::sessions::LoadedHistory {
            messages: vec![json!({"role":"user","content":"previewed"})],
            model: None,
            thinking_level: None,
            pending_question: None,
        }),
    });
    assert!(!owner.snapshot.history_preview);
    owner.apply_history(HistoryResult {
        generation: 2,
        path: new_path.clone(),
        project: new_project.clone(),
        kind: HistoryLoadKind::Selection,
        result: Ok(crate::sessions::LoadedHistory {
            messages: vec![json!({"role":"user","content":"previewed"})],
            model: None,
            thinking_level: None,
            pending_question: Some(crate::sessions::RestoredQuestion {
                id: "restored-question:one".into(),
                title: "Continue?".into(),
                options: Vec::new(),
            }),
        }),
    });

    assert!(owner.process.is_some());
    assert!(owner.snapshot.history_preview);
    assert!(owner.snapshot.pending_question.is_some());
    assert_eq!(owner.snapshot.project, new_project);
    assert_eq!(owner.project, owner.snapshot.project);
    assert_eq!(owner.snapshot.selected_session, Some(new_path.clone()));
    assert_eq!(
        owner
            .parked_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.selected_session.clone()),
        Some(old_path)
    );

    let _ = event_rx.try_iter().count();
    owner.send_prompt(
        format!("session:{}", new_path.display()),
        PromptMode::Normal,
        "continue".into(),
        Vec::new(),
        true,
    );
    assert!(owner.snapshot.history_preview);
    assert_eq!(owner.snapshot.conversation.items[0].text, "previewed");
    assert!(owner.snapshot.pending_question.is_none());
    assert_eq!(owner.active_session, Some(new_path.clone()));
    assert!(owner.deferred_prompt.is_some());
    let resume_events = event_rx.try_iter().collect::<Vec<_>>();
    assert!(
        resume_events
            .iter()
            .all(|event| !matches!(event, RuntimeEvent::PromptResult { accepted: true, .. }))
    );
    assert!(resume_events.iter().any(|event| matches!(
        event,
        RuntimeEvent::SessionReset {
            preserve_submission: true,
            ..
        }
    )));
    assert!(
        resume_events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                _ => None,
            })
            .all(|snapshot| {
                snapshot.history_preview
                    && snapshot.pending_question.is_none()
                    && snapshot
                        .conversation
                        .items
                        .first()
                        .map(|item| item.text.as_str())
                        == Some("previewed")
            })
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        while let Some(item) = owner.process.as_mut().and_then(|process| process.poll()) {
            owner.apply_process_item(item);
        }
        if owner.deferred_prompt.is_none() && owner.pending_prompt_id.is_none() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(owner.deferred_prompt.is_none());
    assert!(owner.pending_prompt_id.is_none());
    assert!(!owner.snapshot.history_preview);
    assert!(owner.snapshot.conversation.items.iter().any(|item| {
        item.kind == crate::app::views::transcript::conversation::TranscriptKind::User
            && item.text == "continue"
    }));
    assert!(event_rx.try_iter().any(|event| matches!(
        event,
        RuntimeEvent::PromptResult {
            target,
            accepted: true,
            ..
        } if target == format!("session:{}", new_path.display())
    )));
    Ok(())
}

#[test]
fn active_session_events_stay_parked_while_other_history_is_visible() -> Result<(), String> {
    let temp = tempdir().map_err(|error| error.to_string())?;
    let active_path = temp.path().join("active.jsonl");
    let history_path = temp.path().join("history.jsonl");
    let history_project = temp.path().join("history-project");
    fs::create_dir(&history_project).map_err(|error| error.to_string())?;
    fs::write(
            &history_path,
            format!(
                "{{\"type\":\"session\",\"id\":\"history\",\"cwd\":{},\"timestamp\":\"2026-08-15T00:00:00Z\"}}\n{{\"type\":\"message\",\"id\":\"one\",\"parentId\":null,\"message\":{{\"role\":\"user\",\"content\":\"history message\"}}}}\n",
                serde_json::to_string(&history_project).map_err(|error| error.to_string())?
            ),
        )
        .map_err(|error| error.to_string())?;

    let (event_tx, event_rx) = test_event_channel();
    let (discovery_tx, _discovery_rx) = mpsc::channel();
    let (history_tx, history_rx) = mpsc::channel();
    let mut owner = RuntimeOwner {
        project: temp.path().to_path_buf(),
        harness: "pi".into(),
        session_id: None,
        process_command: AgentLaunchConfig::default(),
        process: None,
        snapshot: RuntimeSnapshot {
            connected: true,
            status: "Working".into(),
            project: temp.path().to_path_buf(),
            selected_session: Some(active_path.clone()),
            ..RuntimeSnapshot::default()
        },
        owns_session_catalog: false,
        session_generation: 0,
        session_discovery_in_flight: false,
        session_refresh_pending: false,
        session_refresh_due: None,
        process_generation: 7,
        pending_prompt_id: Some("pending-prompt".into()),
        pending_prompt_target: Some(format!("session:{}", active_path.display())),
        pending_prompt_item: None,
        pending_outbox_id: None,
        title_generation: SessionTitleGeneration::default(),
        transcript_changed_from: None,
        event_tx,
        discovery_tx,
        history_tx,
        history_generation: 0,
        history_selection_generation: None,
        document_refresh_generation: None,
        pending_document_refresh: None,
        active_session: Some(active_path.clone()),
        parked_snapshot: None,
        deferred_prompt: None,
        pending_session_controls: PendingSessionControls::default(),
        access_mode_changes: AccessModeChangeState::default(),
        startup_state_loaded: true,
        startup_history_loaded: true,
        state: None,
        session_query: String::new(),
    };
    conversation_mut(&mut owner.snapshot).reduce(&json!({"type":"agent_start"}));

    owner.select_history(history_path.clone(), history_project.clone());
    let history = history_rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| format!("history switch was rejected: {error}"))?;
    owner.apply_history(history);

    assert!(owner.snapshot.history_preview);
    assert_eq!(owner.snapshot.selected_session, Some(history_path));
    assert_eq!(owner.snapshot.conversation.items[0].text, "history message");
    assert!(
        owner
            .parked_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.conversation.running)
    );
    let visible = event_rx
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        })
        .last()
        .expect("history preview should publish");
    assert_eq!(visible.live_session, Some(active_path.clone()));
    assert_eq!(visible.live_status, "Working");
    assert_eq!(visible.conversation.items[0].text, "history message");

    assert_eq!(
        owner.apply_process_item(SessionEvent::Activity(
            json!({
                "type": "message_start",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "active output"}]
                }
            })
            .into(),
        )),
        SnapshotChange::None
    );

    assert_eq!(owner.snapshot.conversation.items[0].text, "history message");
    assert!(owner.parked_snapshot.as_ref().is_some_and(|snapshot| {
        snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.text == "active output")
    }));
    assert!(
        event_rx
            .try_iter()
            .all(|event| !matches!(event, RuntimeEvent::Snapshot { .. }))
    );

    let changed = owner.apply_process_item(SessionEvent::Activity(
        json!({
            "type": "compaction_start",
            "reason": "test"
        })
        .into(),
    ));
    assert_eq!(changed, SnapshotChange::Immediate);
    owner.publish();
    let visible = event_rx
        .try_iter()
        .filter_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        })
        .last()
        .expect("live badge change should publish");
    assert_eq!(visible.live_session, Some(active_path.clone()));
    assert_eq!(visible.live_status, "Compacting");
    assert_eq!(visible.conversation.items[0].text, "history message");

    owner.select_history(active_path.clone(), temp.path().to_path_buf());
    assert!(!owner.snapshot.history_preview);
    assert_eq!(owner.snapshot.selected_session, Some(active_path));
    assert!(
        owner
            .snapshot
            .conversation
            .items
            .iter()
            .any(|item| item.text == "active output")
    );
    Ok(())
}

#[test]
fn refreshing_visible_external_history_preserves_transcript_ui_state() {
    let path = PathBuf::from("/sessions/external.jsonl");
    let project = PathBuf::from("/project");
    let (mut owner, events, _discovery) = owner_without_process(project.clone());
    owner.history_generation = 1;
    preview_history(&mut owner, path.clone(), "before");

    owner.document_refresh_generation = Some(1);
    owner.apply_history(HistoryResult {
        generation: 1,
        path,
        project,
        kind: HistoryLoadKind::DocumentRefresh,
        result: Ok(LoadedHistory {
            messages: vec![json!({"role":"user","content":"after"})],
            model: None,
            thinking_level: None,
            pending_question: None,
        }),
    });

    let published = events.try_iter().collect::<Vec<_>>();
    assert!(published.iter().any(|event| matches!(
        event,
        RuntimeEvent::Snapshot { snapshot, .. }
            if snapshot.conversation.items[0].text == "after"
    )));
    assert!(
        published
            .iter()
            .all(|event| !matches!(event, RuntimeEvent::HistoryReset { .. }))
    );
}

#[test]
fn external_writes_refresh_only_resident_history_documents() {
    let external = PathBuf::from("/sessions/external.jsonl");
    let live = PathBuf::from("/sessions/live.jsonl");
    let project = PathBuf::from("/project");
    let latest = HashMap::from([
        (
            "external".into(),
            Arc::new(RuntimeSnapshot {
                project: project.clone(),
                selected_session: Some(external.clone()),
                history_preview: true,
                ..RuntimeSnapshot::default()
            }),
        ),
        (
            "live".into(),
            Arc::new(RuntimeSnapshot {
                project,
                selected_session: Some(live.clone()),
                history_preview: false,
                ..RuntimeSnapshot::default()
            }),
        ),
    ]);

    assert_eq!(
        changed_external_documents(&latest, &[external.clone(), live]),
        vec![("external".into(), external, PathBuf::from("/project"))]
    );
}

#[test]
fn external_write_during_selection_refreshes_the_newly_loaded_document() {
    let path = PathBuf::from("/sessions/external.jsonl");
    let project = PathBuf::from("/project");
    let (mut owner, _events, _discovery) = owner_without_process(project.clone());
    owner.history_generation = 1;
    owner.history_selection_generation = Some(1);

    owner.refresh_session_document(path.clone(), project.clone());
    owner.apply_history(HistoryResult {
        generation: 1,
        path,
        project,
        kind: HistoryLoadKind::Selection,
        result: Ok(LoadedHistory {
            messages: Vec::new(),
            model: None,
            thinking_level: None,
            pending_question: None,
        }),
    });

    assert_eq!(owner.document_refresh_generation, Some(2));
    assert!(owner.pending_document_refresh.is_none());
    assert_eq!(owner.history_generation, 2);
}

#[test]
fn external_document_refreshes_coalesce_while_a_load_is_in_flight() {
    let path = PathBuf::from("/sessions/external.jsonl");
    let project = PathBuf::from("/project");
    let (mut owner, _events, _discovery) = owner_without_process(project.clone());
    owner.history_generation = 1;
    owner.document_refresh_generation = Some(1);
    preview_history(&mut owner, path.clone(), "before");

    owner.refresh_session_document(path.clone(), project.clone());

    assert_eq!(
        owner.pending_document_refresh,
        Some((path.clone(), project.clone()))
    );
    owner.apply_history(HistoryResult {
        generation: 1,
        path,
        project,
        kind: HistoryLoadKind::DocumentRefresh,
        result: Ok(LoadedHistory {
            messages: Vec::new(),
            model: None,
            thinking_level: None,
            pending_question: None,
        }),
    });
    assert_eq!(owner.document_refresh_generation, Some(2));
    assert!(owner.pending_document_refresh.is_none());
    assert_eq!(owner.history_generation, 2);
}
