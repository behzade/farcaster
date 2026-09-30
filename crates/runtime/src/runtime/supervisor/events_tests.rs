use super::*;
use crate::agents::Backend;

struct SupervisorFixture {
    supervisor: Supervisor,
    commands: mpsc::Sender<RuntimeCommand>,
    events: mpsc::Receiver<RuntimeEvent>,
}

impl SupervisorFixture {
    fn new(selected: &str, project: PathBuf, state: Option<StateStore>) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        let (wake, _) = async_channel::bounded(1);
        let (_, configuration_rx) = mpsc::channel();
        let fixture = Self {
            supervisor: Supervisor {
                host: crate::test_support::host(),
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
                account_usage: HashMap::new(),
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
                catalog_state: state.map(Into::into),
                configuration_catalogs: Vec::new(),
                configuration_rx,
                configuration_tx: None,
                configuration_requests: HashSet::new(),
                requested_access_modes: HashMap::new(),
                published_statuses: HashMap::new(),
            },
            commands,
            events,
        };
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

fn catalog_session(path: &Path, project: &Path, running: bool) -> crate::sessions::SessionSummary {
    crate::sessions::SessionSummary::from_cached(
        "session".into(),
        path.to_path_buf(),
        project.to_path_buf(),
        "Session".into(),
        String::new(),
        String::new(),
        None,
        std::time::SystemTime::now(),
        1,
        crate::sessions::UsageSummary::default(),
        false,
        running,
        String::new(),
    )
}

#[test]
fn selecting_a_retired_session_publishes_the_retained_document_without_actor_commands() {
    let project = PathBuf::from("/project");
    let path = PathBuf::from("/sessions/retired.jsonl");
    let key = format!("session:{}", path.display());
    let mut fixture = SupervisorFixture::new("session:b", project.clone(), None);
    let actor = fixture.add_recording_actor(&key);
    let mut conversation = ConversationState::default();
    conversation.replace_history(&[json!({"role": "assistant", "content": "retained"})]);
    let conversation = Arc::new(conversation);
    let stats = json!({"contextUsage": {"tokens": 100, "contextWindow": 1000, "percent": 10.0}});
    fixture.supervisor.latest.insert(
        key.clone(),
        Arc::new(RuntimeSnapshot {
            harness: Some(Backend::Pi),
            project: project.clone(),
            selected_session: Some(path.clone()),
            live_session: Some(path.clone()),
            document_ready: true,
            conversation: conversation.clone(),
            stats: stats.clone(),
            ..RuntimeSnapshot::default()
        }),
    );
    fixture
        .commands
        .send(RuntimeCommand::SelectSession {
            path,
            harness: Backend::Pi,
            session_id: "retired".into(),
            project,
        })
        .expect("fixture value");
    assert!(fixture.supervisor.process_next_command());
    let displayed = fixture
        .drain()
        .into_iter()
        .find_map(|event| match event {
            RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        })
        .expect("cached document published");
    assert!(Arc::ptr_eq(&displayed.conversation, &conversation));
    assert_eq!(displayed.stats, stats);
    assert!(!displayed.connected);
    fixture.supervisor.actors[&key].send(RuntimeCommand::SystemWake);
    assert!(matches!(
        actor.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::SystemWake)
    ));
    assert_eq!(fixture.supervisor.selected, key);
}

#[test]
fn retirement_refreshes_history_only_when_the_document_revision_changes() {
    let project = PathBuf::from("/project");
    let path = PathBuf::from("/sessions/retired.jsonl");
    let key = format!("session:{}", path.display());
    let mut fixture = SupervisorFixture::new(&key, project.clone(), None);
    let actor = fixture.add_recording_actor(&key);
    fixture.supervisor.interacted.insert(key.clone());
    fixture.supervisor.latest.insert(
        key.clone(),
        Arc::new(RuntimeSnapshot {
            harness: Some(Backend::Pi),
            project: project.clone(),
            selected_session: Some(path.clone()),
            live_session: Some(path.clone()),
            connected: true,
            document_ready: true,
            ..RuntimeSnapshot::default()
        }),
    );
    let mut session = catalog_session(&path, &project, false);
    let reconcile = |fixture: &mut SupervisorFixture, session: &SessionSummary| {
        let supervisor = &mut fixture.supervisor;
        reconcile_live_session_documents(
            std::slice::from_ref(session),
            &supervisor.interacted,
            &supervisor.selected,
            &mut supervisor.actors,
            &mut supervisor.latest,
            &mut supervisor.last_touch,
            &mut supervisor.document_revisions,
            &mut supervisor.actor_paths,
            &supervisor.process_command,
            &supervisor.supervisor_thread,
            &supervisor.host,
        );
    };
    reconcile(&mut fixture, &session);
    Arc::make_mut(
        fixture
            .supervisor
            .latest
            .get_mut(&key)
            .expect("fixture value"),
    )
    .connected = false;
    reconcile(&mut fixture, &session);
    fixture.supervisor.actors[&key].send(RuntimeCommand::SystemWake);
    assert!(matches!(
        actor.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::SystemWake)
    ));
    session.message_count += 1;
    reconcile(&mut fixture, &session);
    assert!(matches!(actor.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::RefreshSessionDocument { path: refreshed, .. }) if refreshed == path));
}

#[test]
fn account_usage_follows_the_profile_and_newest_observation_across_sessions() {
    let mut fixture = SupervisorFixture::new("selected", PathBuf::from("/project"), None);
    let selected = Arc::make_mut(
        fixture
            .supervisor
            .latest
            .get_mut("selected")
            .expect("fixture value"),
    );
    selected.harness = Some(Backend::Codex);
    selected.profile_id = Some("one".into());
    let initial = Instant::now();
    let snapshot = |harness, profile: &str, observed_at, remaining: Option<f64>| {
        Arc::new(RuntimeSnapshot {
            harness: Some(harness),
            profile_id: Some(profile.into()),
            account_usage_observed_at: observed_at,
            account_usage: agents::AccountUsage {
                weekly: remaining.map(|remaining_percent| agents::AccountUsageWindow {
                    remaining_percent,
                    resets_at: Some(1234),
                }),
            },
            ..RuntimeSnapshot::default()
        })
    };
    let send = |fixture: &mut SupervisorFixture, key: &str, snapshot| {
        fixture.supervisor.handle_actor_event(
            key.into(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot,
            },
        )
    };
    send(
        &mut fixture,
        "older",
        snapshot(Backend::Codex, "one", Some(initial), Some(68.0)),
    );
    assert!(fixture.drain().iter().any(|event| matches!(event,
        RuntimeEvent::Snapshot { snapshot, .. } if snapshot.account_usage.weekly.is_some_and(|window| window.remaining_percent == 68.0)
    )));
    send(
        &mut fixture,
        "newer",
        snapshot(
            Backend::Codex,
            "one",
            Some(initial + Duration::from_secs(1)),
            Some(60.0),
        ),
    );
    send(
        &mut fixture,
        "older",
        snapshot(Backend::Codex, "one", Some(initial), Some(68.0)),
    );
    send(
        &mut fixture,
        "unknown",
        snapshot(Backend::Codex, "one", None, None),
    );
    for key in ["selected", "older", "unknown"] {
        assert_eq!(
            fixture.supervisor.latest[key]
                .account_usage
                .weekly
                .map(|window| window.remaining_percent),
            Some(60.0),
            "{key} must use the newest account report"
        );
    }
    for (harness, profile) in [(Backend::Codex, "two"), (Backend::Claude, "one")] {
        send(
            &mut fixture,
            "other",
            snapshot(harness, profile, None, None),
        );
        assert!(
            fixture.supervisor.latest["other"]
                .account_usage
                .weekly
                .is_none()
        );
    }
    send(
        &mut fixture,
        "newer",
        snapshot(
            Backend::Codex,
            "one",
            Some(initial + Duration::from_secs(2)),
            None,
        ),
    );
    assert!(
        fixture.supervisor.latest["selected"]
            .account_usage
            .weekly
            .is_none()
    );
}

#[test]
fn system_wake_reaches_selected_and_background_actors() {
    let mut fixture = SupervisorFixture::new("selected", PathBuf::from("/project"), None);
    let selected = fixture.add_recording_actor("selected");
    let background = fixture.add_recording_actor("background");
    fixture.commands.send(RuntimeCommand::SystemWake).unwrap();
    assert!(fixture.supervisor.process_next_command());
    for actor in [selected, background] {
        assert!(matches!(
            actor.recv_timeout(Duration::from_secs(1)),
            Ok(RuntimeCommand::SystemWake)
        ));
    }
}

#[test]
fn settled_snapshot_clears_stale_catalog_running_for_selected_and_background_sessions() {
    for selected in [true, false] {
        let project = PathBuf::from("/project");
        let path = PathBuf::from("/project/session");
        let key = format!("session:{}", path.display());
        let mut fixture =
            SupervisorFixture::new(if selected { &key } else { "other" }, project.clone(), None);
        fixture
            .supervisor
            .catalog_sessions
            .push(catalog_session(&path, &project, true));
        let mut conversation = crate::conversation::ConversationState::default();
        conversation.reduce(&serde_json::json!({"type":"agent_start"}));
        conversation.reduce(&serde_json::json!({"type":"agent_settled"}));
        let settled = Arc::new(RuntimeSnapshot {
            harness: Some(Backend::Codex),
            project,
            live_session: Some(path.clone()),
            selected_session: Some(path.clone()),
            conversation: Arc::new(conversation),
            ..RuntimeSnapshot::default()
        });
        fixture.supervisor.handle_actor_event(
            key.clone(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: settled.clone(),
            },
        );

        assert!(!fixture.supervisor.catalog_sessions[0].is_running);
        assert!(fixture.drain().iter().any(|event| {
            matches!(event, RuntimeEvent::SessionUpdated(session)
                if session.path == path && !session.is_running)
        }));

        fixture.supervisor.handle_actor_event(
            key.clone(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: settled,
            },
        );
        assert!(
            !fixture
                .drain()
                .iter()
                .any(|event| matches!(event, RuntimeEvent::SessionUpdated(_)))
        );

        let mut delayed = fixture.supervisor.catalog_sessions[0].clone();
        delayed.is_running = true;
        fixture
            .supervisor
            .handle_actor_event("catalog".into(), RuntimeEvent::SessionUpdated(delayed));
        assert!(!fixture.supervisor.catalog_sessions[0].is_running);
        assert!(
            !fixture
                .drain()
                .iter()
                .any(|event| matches!(event, RuntimeEvent::SessionUpdated(_)))
        );

        let mut conversation = crate::conversation::ConversationState::default();
        conversation.reduce(&serde_json::json!({"type":"agent_start"}));
        fixture.supervisor.handle_actor_event(
            key,
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(RuntimeSnapshot {
                    harness: Some(Backend::Codex),
                    project: PathBuf::from("/project"),
                    live_session: Some(path),
                    conversation: Arc::new(conversation),
                    ..RuntimeSnapshot::default()
                }),
            },
        );
        assert!(fixture.supervisor.catalog_sessions[0].is_running);
    }
}

#[test]
fn first_catalog_load_uses_live_running_snapshot() {
    for selected in [true, false] {
        let project = PathBuf::from("/project");
        let path = PathBuf::from("/project/session");
        let key = format!("session:{}", path.display());
        let mut fixture =
            SupervisorFixture::new(if selected { &key } else { "other" }, project.clone(), None);
        let mut conversation = crate::conversation::ConversationState::default();
        conversation.reduce(&serde_json::json!({"type":"agent_start"}));
        fixture.supervisor.handle_actor_event(
            key,
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(RuntimeSnapshot {
                    harness: Some(Backend::Codex),
                    project: project.clone(),
                    live_session: Some(path.clone()),
                    connected: true,
                    conversation: Arc::new(conversation),
                    ..RuntimeSnapshot::default()
                }),
            },
        );
        fixture.drain();
        let session = catalog_session(&path, &project, false);
        fixture.supervisor.handle_actor_event(
            "catalog".into(),
            RuntimeEvent::Sessions {
                generation: 1,
                sessions: vec![session.clone()],
                all_sessions: vec![session],
                activities: None,
            },
        );
        assert!(fixture.supervisor.catalog_sessions[0].is_running);
        assert!(fixture.drain().iter().any(|event| {
            matches!(event, RuntimeEvent::Sessions { sessions, .. }
                if sessions[0].is_running)
        }));
    }
}

#[test]
fn terminal_snapshot_without_settlement_clears_optimistic_running() {
    for status in ["Done", "Stopped", "Failed", "Ready", "Command failed"] {
        let project = PathBuf::from("/project");
        let path = PathBuf::from("/project/session");
        let key = format!("session:{}", path.display());
        let mut fixture = SupervisorFixture::new(&key, project.clone(), None);
        fixture
            .supervisor
            .catalog_sessions
            .push(catalog_session(&path, &project, true));
        fixture.supervisor.handle_actor_event(
            key,
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(RuntimeSnapshot {
                    harness: Some(Backend::Codex),
                    project,
                    live_session: Some(path.clone()),
                    status: status.into(),
                    ..RuntimeSnapshot::default()
                }),
            },
        );
        assert!(
            !fixture.supervisor.catalog_sessions[0].is_running,
            "{status}"
        );
        assert!(fixture.drain().iter().any(|event| {
            matches!(event, RuntimeEvent::SessionUpdated(session)
                if session.path == path && !session.is_running)
        }));
    }
}

#[test]
fn compaction_and_retry_keep_a_settled_session_active_in_catalog() {
    let project = PathBuf::from("/project");
    let path = PathBuf::from("/project/session");
    let key = format!("session:{}", path.display());
    let mut fixture = SupervisorFixture::new(&key, project.clone(), None);
    fixture
        .supervisor
        .catalog_sessions
        .push(catalog_session(&path, &project, false));
    let mut conversation = crate::conversation::ConversationState::default();
    conversation.reduce(&serde_json::json!({"type":"agent_start"}));
    conversation.reduce(&serde_json::json!({"type":"agent_settled"}));
    for (event, active) in [
        ("compaction_start", true),
        ("compaction_end", false),
        ("auto_retry_start", true),
        ("auto_retry_end", false),
    ] {
        conversation.reduce(&serde_json::json!({"type": event}));
        fixture.supervisor.handle_actor_event(
            key.clone(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(RuntimeSnapshot {
                    harness: Some(Backend::Pi),
                    project: project.clone(),
                    live_session: Some(path.clone()),
                    conversation: Arc::new(conversation.clone()),
                    ..RuntimeSnapshot::default()
                }),
            },
        );
        assert_eq!(
            fixture.supervisor.catalog_sessions[0].is_running, active,
            "{event}"
        );
    }
}

#[test]
fn history_preview_reconciles_the_parked_live_status() {
    let project = PathBuf::from("/project");
    let path = PathBuf::from("/project/session");
    let key = format!("session:{}", path.display());
    let mut fixture = SupervisorFixture::new(&key, project.clone(), None);
    fixture
        .supervisor
        .catalog_sessions
        .push(catalog_session(&path, &project, true));
    let preview = |live_status: &str| {
        Arc::new(RuntimeSnapshot {
            harness: Some(Backend::Codex),
            project: project.clone(),
            live_session: Some(path.clone()),
            selected_session: Some(path.clone()),
            history_preview: true,
            live_status: live_status.into(),
            ..RuntimeSnapshot::default()
        })
    };
    fixture.supervisor.handle_actor_event(
        key.clone(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: preview("Done"),
        },
    );
    assert!(!fixture.supervisor.catalog_sessions[0].is_running);

    let mut delayed = fixture.supervisor.catalog_sessions[0].clone();
    delayed.is_running = true;
    fixture
        .supervisor
        .handle_actor_event("catalog".into(), RuntimeEvent::SessionUpdated(delayed));
    assert!(!fixture.supervisor.catalog_sessions[0].is_running);

    fixture.supervisor.handle_actor_event(
        key,
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: preview("Working"),
        },
    );
    assert!(fixture.supervisor.catalog_sessions[0].is_running);
    assert!(fixture.drain().iter().any(|event| {
        matches!(event, RuntimeEvent::SessionStatus { status, .. } if status == "Working")
    }));
}

#[test]
fn capability_only_catalog_reaches_draft_once_without_snapshot_loop() {
    let project = PathBuf::from("/project");
    let mut fixture = SupervisorFixture::new("draft:pi", project.clone(), None);
    fixture.supervisor.configurations.set_catalog(
        Backend::Pi,
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
                harness: Some(Backend::Pi),
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
    let mut fixture = SupervisorFixture::new("draft:open", project.clone(), None);
    fixture.supervisor.latest.insert(
        "draft:open".into(),
        Arc::new(RuntimeSnapshot {
            harness: Some(Backend::OpenCode),
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
    assert_eq!(
        fixture
            .supervisor
            .configurations
            .access_mode(Some(Backend::OpenCode)),
        Some(Sandboxed)
    );

    fixture
        .commands
        .send(RuntimeCommand::LoadConfiguration {
            harness: Backend::OpenCode,
            project: project.clone(),
        })
        .expect("queue catalog load");
    assert!(fixture.supervisor.process_next_command());
    assert_eq!(
        fixture
            .supervisor
            .configuration_process_command(Backend::OpenCode, &project, "draft:open", None)
            .expect("configuration")
            .access_mode,
        Sandboxed
    );
    assert_eq!(
        fixture
            .supervisor
            .configuration_process_command(
                Backend::OpenCode,
                std::path::Path::new("/other"),
                "draft:open",
                None,
            )
            .expect("configuration")
            .access_mode,
        Auto,
        "an unrelated project must not inherit the selected actor policy"
    );
}

#[test]
fn new_session_restores_the_harness_access_mode_before_staging_the_draft() {
    use crate::agents::HarnessAccessMode::Full;

    let project = PathBuf::from("/project");
    let mut fixture = SupervisorFixture::new("draft:old", project.clone(), None);
    assert!(
        fixture
            .supervisor
            .configurations
            .set_access_mode(Some(Backend::Codex), Full)
    );
    let actor_commands = fixture.add_recording_actor("draft:new");
    fixture
        .commands
        .send(RuntimeCommand::NewSession {
            id: "new".into(),
            harness: Some(Backend::Codex),
            profile_id: None,
            project,
        })
        .expect("queue new session");

    assert!(fixture.supervisor.process_next_command());
    assert!(matches!(
        actor_commands.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::RestoreAccessMode(Full))
    ));
    assert!(matches!(
        actor_commands.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::NewSession { .. })
    ));
}

#[test]
fn restart_injects_the_sessions_saved_access_mode_before_launch()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::agents::HarnessAccessMode::{Full, Sandboxed};

    let temp = tempfile::tempdir()?;
    let project = temp.path().to_owned();
    let session = temp.path().join("session-locators/codex-cli/session-1");
    let mut state = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    state.update_session_metadata(&crate::agents::SessionMetadata {
        profile_id: None,
        harness: Backend::Codex,
        id: "session-1".into(),
        path: session.clone(),
        project: project.clone(),
        title: None,
        first_user_message: None,
        parent_session: None,
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: Some(Full),
        usage: None,
        is_running: false,
    })?;
    let key = format!("session:{}", session.display());
    let mut fixture = SupervisorFixture::new(&key, project.clone(), Some(state));
    fixture.supervisor.latest.insert(
        key.clone(),
        Arc::new(RuntimeSnapshot {
            harness: Some(Backend::Codex),
            project: project.clone(),
            selected_session: Some(session.clone()),
            access_mode: Sandboxed,
            ..RuntimeSnapshot::default()
        }),
    );
    let actor_commands = fixture.add_recording_actor(&key);
    fixture.commands.send(RuntimeCommand::RestartSession {
        path: session.clone(),
        harness: Backend::Codex,
        session_id: "session-1".into(),
        project,
    })?;

    assert!(fixture.supervisor.process_next_command());
    assert!(matches!(
        actor_commands.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::RestoreAccessMode(Full))
    ));
    assert!(matches!(
        actor_commands.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::RestartSession { .. })
    ));
    assert_eq!(fixture.supervisor.latest[&key].access_mode, Full);

    fixture
        .commands
        .send(RuntimeCommand::SetAccessMode(Sandboxed))?;
    assert!(fixture.supervisor.process_next_command());
    assert!(matches!(
        actor_commands.recv_timeout(Duration::from_secs(1)),
        Ok(RuntimeCommand::SetAccessMode(Sandboxed))
    ));
    assert_eq!(
        fixture
            .supervisor
            .catalog_state
            .as_ref()
            .expect("catalog state")
            .with(|store| store.session_access_mode(&session))?,
        Some(Sandboxed)
    );
    Ok(())
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
    let mut fixture = SupervisorFixture::new("selected", PathBuf::from("/project"), None);
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
    let mut fixture = SupervisorFixture::new("selected", project.clone(), None);
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
            harness: Backend::Codex,
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
