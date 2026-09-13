use super::*;

struct SupervisorFixture {
    supervisor: Supervisor,
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
}

impl SupervisorFixture {
    fn new(
        selected: &str,
        project: PathBuf,
        state: Option<StateStore>,
        recovery: crate::app::runtime::recovery::InterruptedPromptRecovery,
    ) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let (wake, _) = async_channel::bounded(1);
        let (_, configuration_rx) = mpsc::channel();
        let mut fixture = Self {
            supervisor: Supervisor {
                process_command: AgentLaunchConfig::default(),
                command_rx,
                event_tx: UiEventSender {
                    events: events_tx,
                    wake,
                },
                supervisor_thread: thread::current(),
                catalog_key: "catalog".into(),
                actors: HashMap::new(),
                selected: selected.into(),
                selected_project: project.clone(),
                selected_session: None,
                generation: 0,
                latest: HashMap::from([(
                    selected.into(),
                    Arc::new(RuntimeSnapshot {
                        project,
                        ..RuntimeSnapshot::default()
                    }),
                )]),
                catalog_sessions: Vec::new(),
                catalog_generation: 0,
                actor_paths: HashMap::new(),
                failed_actor_shutdowns: HashMap::new(),
                interacted: HashSet::new(),
                document_revisions: HashMap::new(),
                pending_extensions: HashMap::new(),
                active_dialogs: HashMap::new(),
                needs_input: HashSet::new(),
                clock: 0,
                last_touch: HashMap::new(),
                configurations: HarnessConfigurationStore::default(),
                catalog_state: state,
                configuration_catalogs: Vec::new(),
                configuration_rx,
                configuration_tx: None,
                configuration_requests: HashSet::new(),
                requested_access_modes: HashMap::new(),
                published_statuses: HashMap::new(),
                recovery,
                published_recovery_selection: None,
            },
            commands,
            events,
        };
        fixture.supervisor.publish_recovery_statuses();
        fixture
    }

    fn add_actor(&mut self, key: &str) {
        let (commands, command_rx) = mpsc::channel();
        let (_events_tx, events) = mpsc::channel();
        let join = thread::spawn(move || {
            while let Ok(command) = command_rx.recv() {
                if matches!(command, RuntimeCommand::Shutdown) {
                    break;
                }
            }
            Ok(())
        });
        let actor = SessionRuntimeHandle {
            commands,
            events,
            thread: join.thread().clone(),
            join,
        };
        self.supervisor.actors.insert(key.into(), actor);
    }

    fn add_recording_actor(&mut self, key: &str) -> mpsc::Receiver<RuntimeCommand> {
        let (commands, command_rx) = mpsc::channel();
        let (observed_tx, observed) = mpsc::channel();
        let (_events_tx, events) = mpsc::channel();
        let join = thread::spawn(move || {
            while let Ok(command) = command_rx.recv() {
                if matches!(command, RuntimeCommand::Shutdown) {
                    break;
                }
                let _ = observed_tx.send(command);
            }
            Ok(())
        });
        let actor = SessionRuntimeHandle {
            commands,
            events,
            thread: join.thread().clone(),
            join,
        };
        self.supervisor.actors.insert(key.into(), actor);
        observed
    }

    fn drain(&self) -> Vec<RuntimeEvent> {
        self.events.try_iter().collect()
    }
}

#[test]
fn capability_only_catalog_reaches_draft_once_without_snapshot_loop() {
    let project = PathBuf::from("/project");
    let mut fixture = SupervisorFixture::new("draft:pi", project.clone(), None, Default::default());
    fixture.supervisor.configurations.set_catalog(
        "pi".into(),
        project.clone(),
        crate::agents::ConfigurationCatalog {
            models: vec![],
            efforts: vec!["off".into()],
            sandbox_adapter: Some("pi-nono".into()),
        },
    );
    let commands = fixture.add_recording_actor("draft:pi");

    fixture.supervisor.handle_actor_event(
        "draft:pi".into(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: Arc::new(RuntimeSnapshot {
                harness: "pi".into(),
                project: project.clone(),
                ..RuntimeSnapshot::default()
            }),
        },
    );
    let command = commands
        .recv_timeout(Duration::from_secs(1))
        .expect("supervisor sends the missing capability catalog");
    assert!(matches!(
        &command,
        RuntimeCommand::UpdateConfigurationCatalog { catalog, .. }
            if catalog.models.is_empty()
                && catalog.sandbox_adapter.as_deref() == Some("pi-nono")
    ));

    let (mut actor, actor_events) =
        super::super::super::tests::owner_without_process(project.clone());
    actor.snapshot.connected = false;
    actor.apply_command(command);
    let snapshot = actor_events
        .try_iter()
        .find_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. }
                if snapshot.sandbox_adapter.as_deref() == Some("pi-nono") =>
            {
                Some(snapshot)
            }
            _ => None,
        })
        .expect("actor publishes the applied capability catalog");
    fixture.supervisor.handle_actor_event(
        "draft:pi".into(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot,
        },
    );
    assert!(
        commands.recv_timeout(Duration::from_millis(50)).is_err(),
        "a current capability-only catalog must not be sent back to the actor"
    );
}

#[test]
fn access_mode_command_precedes_catalog_load_when_actor_snapshot_is_delayed() {
    use crate::agents::HarnessAccessMode::{Auto, Full, Sandboxed};

    let project = PathBuf::from("/project");
    let mut fixture =
        SupervisorFixture::new("draft:open", project.clone(), None, Default::default());
    fixture.supervisor.latest.insert(
        "draft:open".into(),
        Arc::new(RuntimeSnapshot {
            harness: "opencode2".into(),
            project: project.clone(),
            access_mode: Full,
            ..RuntimeSnapshot::default()
        }),
    );
    let actor_commands = fixture.add_recording_actor("draft:open");
    fixture
        .commands
        .send(RuntimeCommand::SetAccessMode(Sandboxed))
        .expect("queue selected access mode");
    assert!(fixture.supervisor.process_next_command());
    assert!(matches!(
        actor_commands.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::SetAccessMode(Sandboxed))
    ));
    assert_eq!(fixture.supervisor.latest["draft:open"].access_mode, Full);

    fixture
        .commands
        .send(RuntimeCommand::LoadConfiguration {
            harness: "opencode2".into(),
            project: project.clone(),
        })
        .expect("queue catalog load");
    assert!(fixture.supervisor.process_next_command());
    assert_eq!(
        fixture
            .supervisor
            .configuration_process_command("opencode2", &project, "draft:open")
            .access_mode,
        Sandboxed
    );
    assert_eq!(
        fixture
            .supervisor
            .configuration_process_command(
                "opencode2",
                std::path::Path::new("/other"),
                "draft:open",
            )
            .access_mode,
        Auto,
        "an unrelated project must not inherit the selected actor policy"
    );
}

impl Drop for SupervisorFixture {
    fn drop(&mut self) {
        for actor in self.supervisor.actors.values() {
            actor.send(RuntimeCommand::Shutdown);
        }
        for actor in std::mem::take(&mut self.supervisor.actors).into_values() {
            let _ = actor.join();
        }
    }
}

fn dialog(id: &str) -> ExtensionUiRequest {
    ExtensionUiRequest::Input {
        id: id.into(),
        title: id.into(),
        placeholder: None,
        timeout: None,
    }
}

#[test]
fn selected_dismissal_uses_supervisor_generation_and_clears_needs_input_last() {
    let mut fixture = SupervisorFixture::new(
        "selected",
        PathBuf::from("/project"),
        None,
        Default::default(),
    );
    fixture.supervisor.generation = 8;
    fixture
        .supervisor
        .active_dialogs
        .insert("selected".into(), vec![dialog("first"), dialog("last")]);
    fixture.supervisor.needs_input.insert("selected".into());

    fixture.supervisor.handle_actor_event(
        "selected".into(),
        RuntimeEvent::ExtensionUiDismissed {
            generation: 99,
            id: "first".into(),
        },
    );
    assert!(fixture.supervisor.needs_input.contains("selected"));
    assert_eq!(fixture.supervisor.active_dialogs["selected"].len(), 1);
    assert_eq!(
        fixture.supervisor.published_statuses["selected"].1,
        "Needs input"
    );
    let first = fixture.drain();
    assert!(first.iter().any(|event| matches!(
        event,
        RuntimeEvent::ExtensionUiDismissed { generation: 8, id } if id == "first"
    )));

    fixture.supervisor.handle_actor_event(
        "selected".into(),
        RuntimeEvent::ExtensionUiDismissed {
            generation: 99,
            id: "last".into(),
        },
    );
    assert!(!fixture.supervisor.needs_input.contains("selected"));
    assert!(!fixture.supervisor.active_dialogs.contains_key("selected"));
    assert_ne!(
        fixture.supervisor.published_statuses["selected"].1,
        "Needs input"
    );
    assert!(fixture.drain().iter().any(|event| matches!(
        event,
        RuntimeEvent::ExtensionUiDismissed { generation: 8, id } if id == "last"
    )));
}

#[test]
fn background_dismissal_removes_cached_dialog_before_selection() -> Result<(), String> {
    let project = PathBuf::from("/project");
    let path = PathBuf::from("/sessions/background");
    let key = format!("session:{}", path.display());
    let mut fixture = SupervisorFixture::new("selected", project.clone(), None, Default::default());
    fixture.add_actor(&key);
    fixture
        .supervisor
        .active_dialogs
        .insert(key.clone(), vec![dialog("expired")]);
    fixture.supervisor.needs_input.insert(key.clone());
    fixture.supervisor.handle_actor_event(
        key.clone(),
        RuntimeEvent::ExtensionUiDismissed {
            generation: 0,
            id: "expired".into(),
        },
    );
    assert!(
        fixture.drain().is_empty(),
        "background dismissal must not reach the active UI"
    );

    fixture
        .commands
        .send(RuntimeCommand::SelectSession {
            path,
            harness: "codex-cli".into(),
            session_id: "background".into(),
            project,
        })
        .map_err(|error| error.to_string())?;
    assert!(fixture.supervisor.process_next_command());
    assert!(!fixture.drain().iter().any(|event| matches!(
        event,
        RuntimeEvent::ExtensionUi { request, .. } if request.dialog_id() == Some("expired")
    )));
    Ok(())
}

#[test]
fn cold_selection_keeps_recovery_visible_after_snapshots() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let target = format!("session:{}", session.display());
    let recovered_target = format!(
        "session:{}",
        crate::sessions::normalize_session_path(&session).display()
    );
    let store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        &target,
        "codex-cli",
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "recover this exact prompt",
        &[],
    )?;
    store.begin_prompt(id)?;
    drop(store);
    let state = StateStore::open_at(&database)?;
    let recovery = crate::app::runtime::recovery::InterruptedPromptRecovery::recover(&state)?;
    let mut fixture =
        SupervisorFixture::new("draft:old", PathBuf::from("/old"), Some(state), recovery);
    fixture.add_actor(&target);
    fixture
        .commands
        .send(RuntimeCommand::SelectSession {
            path: session.clone(),
            harness: "codex-cli".into(),
            session_id: "selected".into(),
            project: temp.path().into(),
        })
        .map_err(|error| error.to_string())?;
    assert!(fixture.supervisor.process_next_command());
    assert_eq!(fixture.supervisor.generation, 1);
    assert!(
        !fixture
            .drain()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::ExtensionUi { .. }))
    );

    fixture.supervisor.handle_actor_event(
        target.clone(),
        RuntimeEvent::Snapshot {
            generation: 44,
            snapshot: Arc::new(RuntimeSnapshot {
                project: temp.path().into(),
                harness: "codex-cli".into(),
                selected_session: Some(session.clone()),
                history_preview: true,
                ..RuntimeSnapshot::default()
            }),
        },
    );
    let events = fixture.drain();
    let snapshot_index = events
        .iter()
        .position(|event| matches!(event, RuntimeEvent::Snapshot { generation: 1, .. }))
        .ok_or_else(|| "selected snapshot was not forwarded".to_owned())?;
    let dialog_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                RuntimeEvent::ExtensionUi { generation: 1, request, .. }
                    if request.dialog_id() == Some(format!("farcaster-recovery-{id}").as_str())
            )
        })
        .ok_or_else(|| "recovery dialog was not forwarded".to_owned())?;
    assert!(
        snapshot_index < dialog_index,
        "UI must adopt the generation before the dialog"
    );
    assert_eq!(
        fixture.supervisor.published_statuses[&recovered_target].1,
        "Delivery unknown"
    );

    fixture.supervisor.handle_actor_event(
        target.clone(),
        RuntimeEvent::Snapshot {
            generation: 45,
            snapshot: Arc::new(RuntimeSnapshot {
                project: temp.path().into(),
                harness: "codex-cli".into(),
                selected_session: Some(session.clone()),
                history_preview: true,
                ..RuntimeSnapshot::default()
            }),
        },
    );
    assert_eq!(
        fixture.supervisor.published_statuses[&recovered_target].1, "Delivery unknown",
        "later settled snapshots must not overwrite the recovery blocker"
    );
    assert!(fixture.drain().iter().all(|event| !matches!(
        event,
        RuntimeEvent::SessionStatus { status, .. } if status != "Delivery unknown"
    )));

    fixture
        .commands
        .send(RuntimeCommand::ExtensionResponse(
            ExtensionUiResponse::Cancelled {
                id: format!("farcaster-recovery-{id}"),
                cancelled: true,
            },
        ))
        .map_err(|error| error.to_string())?;
    assert!(fixture.supervisor.process_next_command());
    fixture
        .commands
        .send(RuntimeCommand::SelectSession {
            path: session,
            harness: "codex-cli".into(),
            session_id: "selected".into(),
            project: temp.path().into(),
        })
        .map_err(|error| error.to_string())?;
    assert!(fixture.supervisor.process_next_command());
    assert!(
        fixture.drain().iter().any(|event| matches!(
            event,
            RuntimeEvent::ExtensionUi { generation: 1, request, .. }
                if request.dialog_id() == Some(format!("farcaster-recovery-{id}").as_str())
        )),
        "cancelling and reselecting the same session must show recovery again"
    );
    Ok(())
}

#[test]
fn draft_actor_locator_snapshot_reveals_its_interrupted_prompt() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let target = format!("session:{}", session.display());
    let store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        &target,
        "codex-cli",
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "started from a submitted draft",
        &[],
    )?;
    store.begin_prompt(id)?;
    drop(store);
    let state = StateStore::open_at(&database)?;
    let recovery = crate::app::runtime::recovery::InterruptedPromptRecovery::recover(&state)?;
    let mut fixture =
        SupervisorFixture::new("draft:startup", temp.path().into(), Some(state), recovery);
    fixture.drain();

    fixture.supervisor.handle_actor_event(
        "draft:startup".into(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: Arc::new(RuntimeSnapshot {
                project: temp.path().into(),
                harness: "codex-cli".into(),
                live_session: Some(session),
                ..RuntimeSnapshot::default()
            }),
        },
    );
    let events = fixture.drain();
    let snapshot = events
        .iter()
        .position(|event| matches!(event, RuntimeEvent::Snapshot { generation: 0, .. }))
        .ok_or_else(|| "startup snapshot was not forwarded".to_owned())?;
    let recovery = events
        .iter()
        .position(|event| {
            matches!(
                event,
                RuntimeEvent::ExtensionUi { generation: 0, request, .. }
                    if request.dialog_id() == Some(format!("farcaster-recovery-{id}").as_str())
            )
        })
        .ok_or_else(|| "locator recovery dialog was not forwarded".to_owned())?;
    assert!(snapshot < recovery);
    Ok(())
}

#[test]
fn last_child_dismissal_keeps_unknown_recovery_visible() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let target = format!("session:{}", session.display());
    let recovered_target = format!(
        "session:{}",
        crate::sessions::normalize_session_path(&session).display()
    );
    let store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        &target,
        "codex-cli",
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "unknown delivery beside child input",
        &[],
    )?;
    store.begin_prompt(id)?;
    drop(store);
    let state = StateStore::open_at(&database)?;
    let recovery = crate::app::runtime::recovery::InterruptedPromptRecovery::recover(&state)?;
    let mut fixture = SupervisorFixture::new(&target, temp.path().into(), Some(state), recovery);
    fixture.supervisor.selected_session = Some(session.clone());
    fixture.supervisor.latest.insert(
        target.clone(),
        Arc::new(RuntimeSnapshot {
            project: temp.path().into(),
            harness: "codex-cli".into(),
            selected_session: Some(session),
            ..RuntimeSnapshot::default()
        }),
    );
    fixture.supervisor.active_dialogs.insert(
        target.clone(),
        vec![dialog("farcaster-child-input-expired")],
    );
    fixture.supervisor.needs_input.insert(target.clone());
    fixture.drain();

    fixture.supervisor.handle_actor_event(
        target,
        RuntimeEvent::ExtensionUiDismissed {
            generation: 0,
            id: "farcaster-child-input-expired".into(),
        },
    );
    assert_eq!(
        fixture.supervisor.published_statuses[&recovered_target].1,
        "Delivery unknown"
    );
    assert!(fixture.drain().iter().all(|event| !matches!(
        event,
        RuntimeEvent::SessionStatus { status, .. } if status != "Delivery unknown"
    )));
    Ok(())
}

#[test]
fn selected_reset_allows_recovery_to_publish_after_the_next_snapshot() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let target = format!("session:{}", session.display());
    let store = StateStore::open_at(&database)?;
    let id = store.enqueue_prompt(
        &target,
        "codex-cli",
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "show again after reset",
        &[],
    )?;
    store.begin_prompt(id)?;
    drop(store);
    let state = StateStore::open_at(&database)?;
    let recovery = crate::app::runtime::recovery::InterruptedPromptRecovery::recover(&state)?;
    let mut fixture = SupervisorFixture::new(&target, temp.path().into(), Some(state), recovery);
    fixture.drain();
    let snapshot = Arc::new(RuntimeSnapshot {
        project: temp.path().into(),
        harness: "codex-cli".into(),
        selected_session: Some(session),
        ..RuntimeSnapshot::default()
    });

    fixture.supervisor.handle_actor_event(
        target.clone(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: snapshot.clone(),
        },
    );
    assert!(fixture.drain().iter().any(|event| matches!(
        event,
        RuntimeEvent::ExtensionUi { request, .. }
            if request.dialog_id() == Some(format!("farcaster-recovery-{id}").as_str())
    )));
    fixture.supervisor.handle_actor_event(
        target.clone(),
        RuntimeEvent::SessionReset {
            generation: 0,
            preserve_submission: false,
        },
    );
    assert!(
        fixture
            .drain()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::SessionReset { generation: 0, .. }))
    );
    fixture.supervisor.handle_actor_event(
        target,
        RuntimeEvent::Snapshot {
            generation: 1,
            snapshot,
        },
    );
    assert!(
        fixture.drain().iter().any(|event| matches!(
            event,
            RuntimeEvent::ExtensionUi { request, .. }
                if request.dialog_id() == Some(format!("farcaster-recovery-{id}").as_str())
        )),
        "recovery must publish again after reset cleared the UI"
    );
    Ok(())
}

#[test]
fn recovered_prompts_do_not_block_app_quit_without_live_agents() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session.jsonl");
    let target = format!("session:{}", session.display());
    let state = StateStore::open_at(&database)?;
    let id = state.enqueue_prompt(
        &target,
        "codex-cli",
        temp.path(),
        Some(&session),
        PromptMode::Normal,
        "interrupted prompt",
        &[],
    )?;
    state.begin_prompt(id)?;
    drop(state);
    let state = StateStore::open_at(&database)?;
    let recovery = crate::app::runtime::recovery::InterruptedPromptRecovery::recover(&state)?;
    let mut fixture =
        SupervisorFixture::new("draft:startup", temp.path().into(), Some(state), recovery);
    assert!(fixture.supervisor.actors.is_empty());
    let mut statuses = fixture
        .drain()
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEvent::SessionStatus { target, status, .. } => Some((target, status)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    assert!(
        !statuses.is_empty(),
        "must exercise startup recovery statuses"
    );
    assert!(
        !crate::app::session::activity::application_has_active_work(
            &statuses,
            &RuntimeSnapshot::default(),
            &HashMap::new(),
            &[],
            &[]
        ),
        "saved recovery prompts are not running agents: {statuses:?}"
    );
    // A recovered record must not hide new work in the same session, nor
    // leave an active status behind once that work finishes.
    for status in ["Working", "Compacting", "Retrying", "Needs input", "Done"] {
        let mut snapshot = RuntimeSnapshot {
            project: temp.path().into(),
            selected_session: Some(session.clone()),
            ..Default::default()
        };
        let conversation = Arc::make_mut(&mut snapshot.conversation);
        conversation.running = status == "Working";
        conversation.compacting = status == "Compacting";
        conversation.retrying = status == "Retrying";
        if status == "Needs input" {
            fixture.supervisor.needs_input.insert(target.clone());
        } else {
            fixture.supervisor.needs_input.remove(&target);
        }
        fixture.supervisor.handle_actor_event(
            target.clone(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(snapshot),
            },
        );
        for event in fixture.drain() {
            if let RuntimeEvent::SessionStatus { target, status, .. } = event {
                statuses.insert(target, status);
            }
        }
        assert_eq!(
            crate::app::session::activity::application_has_active_work(
                &statuses,
                &RuntimeSnapshot::default(),
                &HashMap::new(),
                &[],
                &[]
            ),
            status != "Done",
            "quit activity for {status}: {statuses:?}"
        );
    }
    Ok(())
}
