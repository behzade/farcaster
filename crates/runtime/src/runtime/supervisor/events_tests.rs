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
                accounts: super::super::accounts::AccountService::new(None, false),
                host: crate::test_support::host(),
                process_command: AgentLaunchConfig::default(),
                command_rx,
                pending_command: None,
                signal_rx: mpsc::channel().1,
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
                catalog_sessions: Default::default(),
                catalog_generation: 0,
                actor_paths: Default::default(),
                failed_actor_shutdowns: HashMap::new(),
                failed_draft_shutdowns: HashMap::new(),
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
            signals: mpsc::channel().0,
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
            signals: mpsc::channel().0,
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
            display_stamp: None,
            path,
            harness: Backend::Pi,
            session_id: "retired".into(),
            project,
            key: None,
            request: None,
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
            &supervisor.catalog_sessions,
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
fn system_wake_bursts_broadcast_once_to_selected_and_background_actors() {
    let mut fixture = SupervisorFixture::new("selected", PathBuf::from("/project"), None);
    let actors = ["selected", "background"].map(|key| {
        fixture.add_actor(key);
        let (sender, receiver) = mpsc::channel();
        fixture.supervisor.actors.get_mut(key).unwrap().commands = sender;
        receiver
    });
    for _ in 0..3 {
        fixture.commands.send(RuntimeCommand::SystemWake).unwrap();
    }
    assert!(fixture.supervisor.process_next_command());
    for actor in &actors {
        assert!(matches!(actor.try_recv(), Ok(RuntimeCommand::SystemWake)));
        assert!(matches!(actor.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }
    fixture.commands.send(RuntimeCommand::SystemWake).unwrap();
    assert!(fixture.supervisor.process_next_command());
    for actor in &actors {
        assert!(matches!(actor.try_recv(), Ok(RuntimeCommand::SystemWake)));
        assert!(matches!(actor.try_recv(), Err(mpsc::TryRecvError::Empty)));
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
            harness: Some(Backend::Pi),
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
                    harness: Some(Backend::Pi),
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
                    harness: Some(Backend::Pi),
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
                    harness: Some(Backend::Pi),
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
    let preview = |live_status: RunStatus| {
        Arc::new(RuntimeSnapshot {
            harness: Some(Backend::Pi),
            project: project.clone(),
            live_session: Some(path.clone()),
            selected_session: Some(path.clone()),
            history_preview: true,
            live_status,
            ..RuntimeSnapshot::default()
        })
    };
    fixture.supervisor.handle_actor_event(
        key.clone(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: preview(RunStatus::Done),
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
            snapshot: preview(RunStatus::Working),
        },
    );
    assert!(fixture.supervisor.catalog_sessions[0].is_running);
    assert!(fixture.drain().iter().any(|event| {
        matches!(event, RuntimeEvent::SessionStatus { status, .. } if *status == RunStatus::Working)
    }));
}

#[test]
fn draft_snapshot_cannot_publish_working_for_a_sibling_that_arrives_first() {
    let project = PathBuf::from("/project");
    let path = project.join("profiles/11111111-1111-4111-8111-111111111111/codex-cli/shared");
    let actor = "draft:owner";
    let mut fixture = SupervisorFixture::new(actor, project.clone(), None);
    let sibling = catalog_session(&path, &project, false).with_app_session_id(41);
    fixture.supervisor.catalog_sessions.push(sibling.clone());
    let mut conversation = ConversationState::default();
    conversation.reduce(&json!({"type": "agent_start"}));
    let snapshot = Arc::new(RuntimeSnapshot {
        project: project.clone(),
        harness: Some(Backend::Codex),
        profile_id: sessions::profile_id_from_locator(&path),
        selected_session: Some(path.clone()),
        live_session: Some(path.clone()),
        live_status: RunStatus::Working,
        conversation: Arc::new(conversation),
        ..Default::default()
    });
    fixture.supervisor.handle_actor_event(
        actor.into(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: snapshot.clone(),
        },
    );
    let events = fixture.drain();
    assert!(events.iter().any(|event| matches!(event,
        RuntimeEvent::SessionStatus { target, session: Some(saved), record: None, status: RunStatus::Working }
            if target == actor && saved == &path
    )));
    assert!(events.iter().any(|event| matches!(event,
        RuntimeEvent::Snapshot { snapshot, .. }
            if !snapshot.matches_catalog_row(&fixture.supervisor.catalog_sessions, &sibling)
    )));
    assert!(!fixture.supervisor.catalog_sessions[0].is_running);
    fixture.supervisor.handle_actor_event(
        "catalog".into(),
        RuntimeEvent::SessionUpdated(sibling.clone()),
    );
    assert!(!fixture.supervisor.catalog_sessions[0].is_running);

    let mut owner = sibling.with_app_session_id(42);
    owner.harness = Backend::Codex;
    owner.profile_id = None;
    fixture.supervisor.catalog_sessions = vec![owner.clone()].into();
    fixture.supervisor.handle_actor_event(
        actor.into(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot,
        },
    );
    assert!(fixture.supervisor.catalog_sessions[0].is_running);
    assert!(fixture.drain().iter().any(|event| matches!(event,
        RuntimeEvent::SessionStatus { target, record: Some(record), status: RunStatus::Working, .. }
            if target == actor && record == &SessionStatusRecord::for_row(&owner)
    )));
}

#[test]
fn resident_status_keeps_its_record_when_another_session_shares_the_path() {
    let project = PathBuf::from("/project");
    let path = project.join("shared.jsonl");
    let actor = sessions::session_target(&path);
    for (id, same_scope) in [(0, false), (41, false), (41, true)] {
        let a = catalog_session(&path, &project, false).with_app_session_id(id);
        let mut b = a.clone().with_app_session_id(42);
        b.profile_id = (!same_scope).then(|| "other".into());
        let owner = SessionStatusRecord::for_row(&a);
        let mut fixture = SupervisorFixture::new("foreground", project.clone(), None);
        fixture.supervisor.catalog_sessions = vec![a.clone()].into();
        let snapshot = |status| RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: Arc::new(RuntimeSnapshot {
                project: project.clone(),
                harness: Some(a.harness),
                live_session: Some(path.clone()),
                selected_session: Some(path.clone()),
                history_preview: true,
                live_status: status,
                ..RuntimeSnapshot::default()
            }),
        };
        let expect_status = |fixture: &SupervisorFixture, expected| {
            assert!(fixture.drain().iter().any(|e| matches!(e,
                RuntimeEvent::SessionStatus {target, record: Some(record), status, ..}
                    if target == &actor && record == &owner && *status == expected
            )));
        };
        fixture
            .supervisor
            .handle_actor_event(actor.clone(), snapshot(RunStatus::Working));
        expect_status(&fixture, RunStatus::Working);
        fixture
            .supervisor
            .handle_actor_event("catalog".into(), RuntimeEvent::SessionUpdated(b.clone()));
        assert!(
            !fixture
                .supervisor
                .catalog_sessions
                .by_app_id(42)
                .unwrap()
                .is_running
        );
        fixture.supervisor.handle_actor_event(
            actor.clone(),
            RuntimeEvent::ExtensionUi {
                generation: 0,
                request: dialog("question"),
                system_notification_target: None,
            },
        );
        expect_status(&fixture, RunStatus::NeedsInput);
        fixture.supervisor.handle_actor_event(
            actor.clone(),
            RuntimeEvent::ExtensionUiDismissed {
                generation: 0,
                id: "question".into(),
            },
        );
        expect_status(&fixture, RunStatus::Working);
        fixture
            .supervisor
            .handle_actor_event(actor.clone(), snapshot(RunStatus::Done));
        expect_status(&fixture, RunStatus::Done);
        assert!(
            fixture
                .supervisor
                .catalog_sessions
                .running()
                .next()
                .is_none()
        );
    }
}

#[test]
fn an_unknown_resident_cannot_stop_either_row_at_an_ambiguous_locator() {
    let project = PathBuf::from("/project");
    let path = project.join("shared.jsonl");
    let a = catalog_session(&path, &project, true).with_app_session_id(41);
    let b = a.clone().with_app_session_id(42);
    let mut fixture = SupervisorFixture::new("foreground", project.clone(), None);
    fixture.supervisor.catalog_sessions = vec![a.clone(), b].into();
    fixture.supervisor.handle_actor_event(
        sessions::session_target(&path),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: Arc::new(RuntimeSnapshot {
                project,
                harness: Some(a.harness),
                live_session: Some(path.clone()),
                selected_session: Some(path),
                history_preview: true,
                live_status: RunStatus::Stopped,
                ..RuntimeSnapshot::default()
            }),
        },
    );
    assert_eq!(fixture.supervisor.catalog_sessions.running().count(), 2);
    assert!(
        !fixture
            .drain()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::SessionStatus { .. }))
    );
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
fn reconfiguring_a_resident_draft_waits_for_its_actors_profile_snapshot() {
    let project = PathBuf::from("/project");
    let key = "draft:profile-switch";
    let mut fixture = SupervisorFixture::new("catalog", project.clone(), None);
    for (sequence, profile) in [(1, "first"), (2, "second")] {
        fixture
            .commands
            .send(RuntimeCommand::NewSession {
                request: Some(sequence),
                id: "profile-switch".into(),
                harness: Some(Backend::Codex),
                profile_id: Some(profile.into()),
                project: project.clone(),
            })
            .unwrap();
        assert!(fixture.supervisor.process_next_command());
        let events = fixture.drain();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::SelectionRouted(id) if *id == sequence))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::Snapshot { .. }))
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let event = fixture.supervisor.actors[key]
                .events
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("actor publishes its requested profile");
            fixture.supervisor.handle_actor_event(key.into(), event);
            if fixture.drain().iter().any(|event| {
                matches!(event, RuntimeEvent::Snapshot { snapshot, .. }
                if snapshot.profile_id.as_deref() == Some(profile))
            }) {
                break;
            }
        }
        assert_eq!(
            fixture.supervisor.latest[key].profile_id.as_deref(),
            Some(profile)
        );
    }
}

#[test]
fn acknowledged_draft_navigation_delivers_the_next_prompt_to_that_actor() {
    let project = PathBuf::from("/project");
    let mut fixture = SupervisorFixture::new("draft:prior", project.clone(), None);
    for (request, id, command) in [
        (
            1,
            "new",
            RuntimeCommand::NewSession {
                request: Some(1),
                id: "new".into(),
                harness: Some(Backend::Codex),
                profile_id: None,
                project: project.clone(),
            },
        ),
        (
            2,
            "resumed",
            RuntimeCommand::ResumeDraft {
                request: Some(2),
                id: "resumed".into(),
                harness: Some(Backend::Codex),
                project,
            },
        ),
    ] {
        let target = sessions::draft_target(id);
        let actor = fixture.add_recording_actor(&target);
        fixture.commands.send(command).unwrap();
        assert!(fixture.supervisor.process_next_command());
        assert!(
            fixture.drain().iter().any(
                |event| matches!(event, RuntimeEvent::SelectionRouted(seq) if *seq == request)
            )
        );
        fixture
            .commands
            .send(RuntimeCommand::Prompt {
                submission_id: format!("input-{id}"),
                target: target.clone(),
                mode: PromptMode::Normal,
                message: id.to_owned(),
                provisional_title: None,
                display_message: None,
                invocation: None,
                images: Vec::new(),
                allow_while_running: false,
            })
            .unwrap();
        assert!(fixture.supervisor.process_next_command());
        loop {
            if let RuntimeCommand::Prompt {
                target: received,
                message,
                ..
            } = actor
                .recv_timeout(Duration::from_secs(1))
                .expect("draft actor receives the prompt")
            {
                assert_eq!(received, target);
                assert_eq!(message, id);
                break;
            }
        }
    }
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
            request: None,
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
        forked_from: None,
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
        key: None,
        request: None,
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
fn concurrent_questions_do_not_block_and_resolving_approval_restores_working_status() {
    let mut fixture = SupervisorFixture::new("selected", PathBuf::from("/project"), None);
    let snapshot = Arc::make_mut(fixture.supervisor.latest.get_mut("selected").unwrap());
    Arc::make_mut(&mut snapshot.conversation).running = true;
    let request = ExtensionUiRequest::Questions(agents::QuestionRequest {
        id: "async".into(),
        blocking: false,
        questions: vec![],
    });
    fixture.supervisor.handle_actor_event(
        "selected".into(),
        RuntimeEvent::ExtensionUi {
            generation: 0,
            request,
            system_notification_target: None,
        },
    );
    assert!(!fixture.supervisor.needs_input.contains("selected"));
    assert_eq!(
        fixture.supervisor.published_statuses["selected"].1,
        RunStatus::Working
    );
    fixture.supervisor.handle_actor_event(
        "selected".into(),
        RuntimeEvent::ExtensionUi {
            generation: 0,
            request: dialog("blocking"),
            system_notification_target: None,
        },
    );
    assert!(fixture.supervisor.needs_input.contains("selected"));
    fixture.supervisor.handle_actor_event(
        "selected".into(),
        RuntimeEvent::ExtensionUiDismissed {
            generation: 0,
            id: "blocking".into(),
        },
    );
    assert!(!fixture.supervisor.needs_input.contains("selected"));
    assert_eq!(
        fixture.supervisor.published_statuses["selected"].1,
        RunStatus::Working
    );
    assert_eq!(fixture.supervisor.active_dialogs["selected"].len(), 1);
    assert_eq!(
        fixture.supervisor.active_dialogs["selected"][0].dialog_id(),
        Some("async")
    );
}

#[test]
fn answering_a_concurrent_question_does_not_restart_a_finished_session() {
    let mut fixture = SupervisorFixture::new("selected", PathBuf::from("/project"), None);
    let actor = fixture.add_recording_actor("selected");
    fixture.supervisor.active_dialogs.insert(
        "selected".into(),
        vec![ExtensionUiRequest::Questions(agents::QuestionRequest {
            id: "async".into(),
            blocking: false,
            questions: vec![],
        })],
    );
    let mut snapshot = (**fixture.supervisor.latest.get("selected").unwrap()).clone();
    Arc::make_mut(&mut snapshot.conversation).settled = true;
    fixture.supervisor.handle_actor_event(
        "selected".into(),
        RuntimeEvent::Snapshot {
            generation: 0,
            snapshot: Arc::new(snapshot),
        },
    );
    assert_eq!(fixture.supervisor.active_dialogs["selected"].len(), 1);
    fixture
        .commands
        .send(RuntimeCommand::ExtensionResponse(
            ExtensionUiResponse::Questions(agents::QuestionResponse {
                id: "async".into(),
                answers: Default::default(),
                cancelled: true,
            }),
        ))
        .unwrap();
    assert!(fixture.supervisor.process_next_command());
    assert_eq!(
        fixture.supervisor.published_statuses["selected"].1,
        RunStatus::Done
    );
    assert!(!fixture.supervisor.active_dialogs.contains_key("selected"));
    assert!(matches!(
        actor.recv_timeout(Duration::from_secs(1)).unwrap(),
        RuntimeCommand::ExtensionResponse(ExtensionUiResponse::Questions(_))
    ));
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
        RunStatus::NeedsInput
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
        RunStatus::NeedsInput
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
            display_stamp: None,
            path,
            harness: Backend::Codex,
            session_id: "background".into(),
            project,
            key: None,
            request: None,
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
fn signals_keep_their_target_across_selection_changes_and_use_the_signal_channel() {
    let mut fixture = SupervisorFixture::new("draft:new", "/project".into(), None);
    fixture.add_actor("draft:old");
    fixture.add_actor("draft:new");
    let (old_tx, old_rx) = mpsc::channel();
    let (new_tx, new_rx) = mpsc::channel();
    fixture
        .supervisor
        .actors
        .get_mut("draft:old")
        .unwrap()
        .signals = old_tx;
    fixture
        .supervisor
        .actors
        .get_mut("draft:new")
        .unwrap()
        .signals = new_tx;
    fixture
        .supervisor
        .actor_paths
        .insert("/sessions/old".into(), Backend::Pi, "draft:old".into());
    let (signals, receiver) = mpsc::channel();
    fixture.supervisor.signal_rx = receiver;
    signals
        .send(("session:/sessions/old".into(), RuntimeSignal::ApplySteering))
        .unwrap();
    signals
        .send(("draft:old".into(), RuntimeSignal::Abort))
        .unwrap();
    signals
        .send(("draft:missing".into(), RuntimeSignal::Abort))
        .unwrap();
    thread::current().unpark();
    assert!(fixture.supervisor.process_next_command());
    assert_eq!(
        old_rx.try_iter().collect::<Vec<_>>(),
        [RuntimeSignal::ApplySteering, RuntimeSignal::Abort]
    );
    assert!(new_rx.try_recv().is_err());
    assert!(!fixture.supervisor.actors.contains_key("draft:missing"));
}

#[test]
fn composer_stop_keeps_its_actor_when_residents_share_a_path() {
    let path = PathBuf::from("/sessions/shared");
    let mut fixture = SupervisorFixture::new("draft:pi", "/project".into(), None);
    let mut receivers = Vec::new();
    for (key, harness) in [("draft:pi", Backend::Pi), ("draft:codex", Backend::Codex)] {
        fixture.add_actor(key);
        let (sender, receiver) = mpsc::channel();
        fixture.supervisor.actors.get_mut(key).unwrap().signals = sender;
        receivers.push(receiver);
        fixture.supervisor.handle_actor_event(
            key.into(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(RuntimeSnapshot {
                    harness: Some(harness),
                    selected_session: Some(path.clone()),
                    ..Default::default()
                }),
            },
        );
    }
    assert_eq!(
        fixture
            .supervisor
            .actor_paths
            .last_at(&path)
            .map(String::as_str),
        Some("draft:codex")
    );
    let (signals, receiver) = mpsc::channel();
    fixture.supervisor.signal_rx = receiver;
    let pi = sessions::session_buffer(Backend::Pi, &path);
    for selected in ["draft:pi", "draft:codex"] {
        signals.send((pi.clone(), RuntimeSignal::Abort)).unwrap();
        fixture.supervisor.selected = selected.into();
        thread::current().unpark();
        assert!(fixture.supervisor.process_next_command());
        assert_eq!(receivers[0].try_recv(), Ok(RuntimeSignal::Abort));
        assert!(receivers[1].try_recv().is_err());
    }
}

#[test]
fn same_path_selection_routes_the_requested_backend_and_profile() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = project.path().join("shared.jsonl");
    let project = sessions::normalize_session_path(project.path());
    let path = sessions::normalize_session_path(&path);
    let mut pi = catalog_session(&path, &project, false);
    pi.profile_id = Some("pi-profile".into());
    let mut codex = pi.clone();
    codex.harness = Backend::Codex;
    codex.profile_id = Some("codex-profile".into());
    let mut state = StateStore::open_at(&project.join("state.sqlite3"))?;
    state.replace_sessions(&[pi, codex])?;
    let rows = state.cached_sessions("")?;
    assert_eq!(rows.len(), 2);
    let mut fixture = SupervisorFixture::new("catalog", project.clone(), Some(state));
    for row in &rows {
        let key = sessions::AppSessionId::new(row.app_session_id)
            .unwrap()
            .to_key();
        fixture.add_actor(&key);
        fixture.supervisor.latest.insert(
            key.clone(),
            Arc::new(RuntimeSnapshot {
                harness: Some(row.harness),
                project: project.clone(),
                profile_id: row.profile_id.clone(),
                selected_session: Some(path.clone()),
                document_ready: true,
                ..Default::default()
            }),
        );
        fixture
            .supervisor
            .actor_paths
            .insert(path.clone(), row.harness, key);
    }
    let mut sequence = 0;
    for visible in [&rows[..0], &rows[..1], &rows[1..], &rows[..]] {
        fixture.supervisor.catalog_sessions = visible.to_vec().into();
        for row in &rows {
            sequence += 1;
            fixture
                .commands
                .send(RuntimeCommand::SelectSession {
                    display_stamp: None,
                    path: path.clone(),
                    project: project.clone(),
                    harness: row.harness,
                    session_id: row.id.clone(),
                    key: Some(row.catalog_key()),
                    request: Some(sequence),
                })
                .unwrap();
            assert!(fixture.supervisor.process_next_command());
            let events = fixture.drain();
            let ack = events
                .iter()
                .position(|e| matches!(e,RuntimeEvent::SelectionRouted(s) if *s==sequence))
                .unwrap();
            let snapshot = events[ack + 1..]
                .iter()
                .find_map(|e| match e {
                    RuntimeEvent::Snapshot { snapshot, .. } => Some(snapshot),
                    _ => None,
                })
                .unwrap();
            assert!(snapshot.matches_session_row(row));
            assert_eq!(
                fixture.supervisor.selected,
                sessions::AppSessionId::new(row.app_session_id)
                    .unwrap()
                    .to_key()
            );
        }
    }
    Ok(())
}

#[test]
fn cold_selection_reserves_each_backend_actor_before_its_first_snapshot() {
    let project = PathBuf::from("/project");
    let path = project.join("shared.jsonl");
    let pi = catalog_session(&path, &project, false);
    let mut codex = pi.clone();
    codex.harness = Backend::Codex;
    let expect_selection = |commands: &mpsc::Receiver<RuntimeCommand>, expected| {
        assert!(matches!(
            commands.recv_timeout(Duration::from_secs(1)),
            Ok(RuntimeCommand::SelectSession { harness, .. }) if harness == expected
        ));
    };
    for rows in [vec![], vec![pi.clone()], vec![codex.clone()]] {
        let mut fixture = SupervisorFixture::new("catalog", project.clone(), None);
        fixture.supervisor.catalog_sessions = rows.into();
        let select = |fixture: &mut SupervisorFixture, row: &SessionSummary, sequence| {
            fixture
                .commands
                .send(RuntimeCommand::SelectSession {
                    display_stamp: None,
                    path: path.clone(),
                    project: project.clone(),
                    harness: row.harness,
                    session_id: row.id.clone(),
                    key: None,
                    request: Some(sequence),
                })
                .unwrap();
            assert!(fixture.supervisor.process_next_command());
            fixture.drain()
        };
        let pi_key = sessions::session_target(&path);
        let pi_commands = fixture.add_recording_actor(&pi_key);
        select(&mut fixture, &pi, 1);
        expect_selection(&pi_commands, Backend::Pi);
        let codex_key = fixture.supervisor.actor_paths.session_key(
            &path,
            codex.harness,
            &fixture.supervisor.latest,
        );
        assert_ne!(pi_key, codex_key);
        let codex_commands = fixture.add_recording_actor(&codex_key);
        select(&mut fixture, &codex, 2);
        assert_eq!(fixture.supervisor.selected, codex_key);
        expect_selection(&codex_commands, Backend::Codex);
        select(&mut fixture, &pi, 3);
        assert_eq!(fixture.supervisor.selected, pi_key);
        expect_selection(&pi_commands, Backend::Pi);

        fixture.supervisor.handle_actor_event(
            pi_key.clone(),
            RuntimeEvent::Snapshot {
                generation: 0,
                snapshot: Arc::new(RuntimeSnapshot {
                    selected_session: Some(path.clone()),
                    project: project.clone(),
                    harness: Some(Backend::Pi),
                    document_ready: true,
                    ..Default::default()
                }),
            },
        );
        fixture.drain();
        let events = select(&mut fixture, &codex, 4);
        assert_eq!(fixture.supervisor.selected, codex_key);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::SelectionRouted(4)))
        );
        assert!(!events.iter().any(|e| matches!(e, RuntimeEvent::Snapshot { snapshot, .. } if snapshot.harness == Some(Backend::Pi))));
        expect_selection(&codex_commands, Backend::Codex);
    }
}

#[test]
fn archive_acknowledgment_keeps_warm_indexes_and_reconciles_only_the_requested_document() {
    let project = PathBuf::from("/project");
    let path = project.join("shared.jsonl");
    let row = catalog_session(&path, &project, false).with_app_session_id(41);
    let mut other = row.clone().with_app_session_id(42);
    other.harness = Backend::Codex;
    let mut fixture = SupervisorFixture::new("selected", project, None);
    let actor = documents::record_actor_key(&row.catalog_key());
    let other_actor = documents::record_actor_key(&other.catalog_key());
    fixture.add_actor(&actor);
    fixture.add_actor(&other_actor);
    fixture.supervisor.catalog_sessions = vec![row.clone(), other.clone()].into();
    assert!(fixture.supervisor.catalog_sessions.has_shared_path(&path));
    let builds = fixture
        .supervisor
        .catalog_sessions
        .relationship_build_count();
    for archived in [true, true, false] {
        let mut stored = row.clone();
        stored.archived = archived;
        stored.title = "stale storage title".into();
        fixture.supervisor.handle_actor_event(
            "catalog".into(),
            RuntimeEvent::SessionArchiveResult {
                request: ArchiveRequest {
                    target: ArchiveTarget::App(sessions::AppSessionId::new(41).unwrap()),
                    archived,
                    sequence: Some(1),
                },
                rows: Ok(vec![stored]),
            },
        );
        assert_eq!(
            fixture
                .supervisor
                .catalog_sessions
                .relationship_build_count(),
            builds
        );
        let target = fixture.supervisor.catalog_sessions.by_app_id(41).unwrap();
        assert_eq!(target.archived, archived);
        assert_eq!(target.title, row.title);
        assert_eq!(
            fixture.supervisor.catalog_sessions.by_app_id(42),
            Some(&other)
        );
        assert!(!fixture.supervisor.actors.contains_key(&actor));
        assert!(fixture.supervisor.actors.contains_key(&other_actor));
        assert!(matches!(
            fixture.drain().as_slice(),
            [RuntimeEvent::SessionArchiveResult { .. }]
        ));
    }
}

#[cfg(unix)]
#[test]
fn archive_acknowledgment_resolves_the_requested_legacy_locator_and_adopts_its_id() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let alias = project.join("alias");
    std::os::unix::fs::symlink(&project, &alias).unwrap();
    let legacy = catalog_session(&alias.join("session.jsonl"), &alias, false);
    let mut other = legacy.clone();
    other.harness = Backend::Codex;
    let mut stored = legacy.clone().with_app_session_id(92);
    stored.project = project.clone();
    stored.path = project.join("session.jsonl");
    let mut fixture = SupervisorFixture::new("selected", project, None);
    fixture.supervisor.catalog_sessions = vec![legacy.clone(), other.clone()].into();
    for archived in [true, false] {
        stored.archived = archived;
        let target = if archived {
            ArchiveTarget::Native(legacy.target())
        } else {
            ArchiveTarget::App(sessions::AppSessionId::new(92).unwrap())
        };
        fixture.supervisor.handle_actor_event(
            "catalog".into(),
            RuntimeEvent::SessionArchiveResult {
                request: ArchiveRequest {
                    target,
                    archived,
                    sequence: None,
                },
                rows: Ok(vec![stored.clone()]),
            },
        );
        assert_eq!(fixture.supervisor.catalog_sessions.len(), 2);
        let updated = fixture.supervisor.catalog_sessions.by_app_id(92).unwrap();
        assert_eq!(updated.path, legacy.path);
        assert_eq!(updated.archived, archived);
        assert_eq!(fixture.supervisor.catalog_sessions[1], other);
        assert!(matches!(
            fixture.drain().as_slice(),
            [RuntimeEvent::SessionArchiveResult { .. }]
        ));
    }
}
