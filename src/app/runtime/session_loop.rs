use super::*;

pub(super) fn run(
    project: PathBuf,
    process_command: AgentLaunchConfig,
    command_rx: mpsc::Receiver<RuntimeCommand>,
    event_tx: SessionEventSender,
    load_catalog: bool,
) {
    let (discovery_tx, discovery_rx) = mpsc::channel();
    let (history_tx, history_rx) = mpsc::channel();
    let (watch_tx, watch_rx) = mpsc::channel();
    let (session_watcher, watcher_error) = if load_catalog {
        match configured_session_root()
            .and_then(|root| SessionWatcher::start(&root, watch_tx, thread::current()))
        {
            Ok(watcher) => (Some(watcher), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };
    let (state, state_error) = match StateStore::open() {
        Ok(state) => (Some(state), None),
        Err(error) => (None, Some(error)),
    };
    let mut owner = RuntimeOwner {
        project: project.clone(),
        harness: "pi".into(),
        session_id: None,
        process_command,
        process: None,
        snapshot: RuntimeSnapshot {
            status: "Done".into(),
            project,
            auto_retry: true,
            ..RuntimeSnapshot::default()
        },
        owns_session_catalog: load_catalog,
        session_generation: 0,
        session_discovery_in_flight: false,
        session_refresh_pending: false,
        session_refresh_due: None,
        process_generation: 0,
        pending_prompt_id: None,
        pending_prompt_target: None,
        pending_prompt_item: None,
        pending_outbox_id: None,
        title_generation: SessionTitleGeneration::default(),
        transcript_changed_from: Some(0),
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
        state,
        session_query: String::new(),
    };
    if let Some(error) = state_error {
        conversation_mut(&mut owner.snapshot).push_local_error("State unavailable", error);
    }
    if load_catalog {
        owner.load_sessions(String::new());
    }
    if let Some(message) = watcher_error {
        let _ = owner.event_tx.send(RuntimeEvent::SessionsFailed {
            generation: owner.session_generation,
            message,
        });
    }
    owner.publish();
    let _session_watcher = session_watcher;
    let mut running = true;
    let mut stream_publish_due = None;
    while running {
        while let Ok(result) = discovery_rx.try_recv() {
            owner.apply_discovery(result);
        }
        while let Ok(result) = history_rx.try_recv() {
            owner.apply_history(result);
        }
        while let Ok(result) = owner.title_generation.receiver.try_recv() {
            owner.apply_generated_session_title(result);
        }
        while let Ok(event) = watch_rx.try_recv() {
            match event {
                SessionWatchEvent::CatalogChanged => owner.schedule_session_refresh(),
                SessionWatchEvent::Activity(paths) => {
                    let _ = owner
                        .event_tx
                        .send(RuntimeEvent::SessionFilesModified { paths });
                }
                SessionWatchEvent::Failed(message) => {
                    let _ = owner.event_tx.send(RuntimeEvent::SessionsFailed {
                        generation: owner.session_generation,
                        message,
                    });
                }
            }
        }
        owner.poll_deferred_session_refresh(Instant::now());
        let mut immediate_snapshot_change = false;
        while let Some(item) = owner.process.as_mut().and_then(|process| process.poll()) {
            match owner.apply_process_item(item) {
                SnapshotChange::None => {}
                SnapshotChange::Streaming => {
                    let coalesced = stream_publish_due.is_some();
                    crate::app::infrastructure::performance::count_stream_event(coalesced);
                    if !coalesced {
                        stream_publish_due = Some(Instant::now() + STREAM_PUBLISH_INTERVAL);
                    }
                }
                SnapshotChange::Immediate => immediate_snapshot_change = true,
            }
        }
        owner.apply_queued_access_mode_change();
        if immediate_snapshot_change
            || stream_publish_due.is_some_and(|deadline| Instant::now() >= deadline)
        {
            owner.publish();
            stream_publish_due = None;
        }
        let now = Instant::now();
        let access_mode_change_due = owner
            .access_mode_change_ready()
            .then(|| owner.access_mode_changes.next_deadline())
            .flatten();
        let next_deadline = [
            stream_publish_due,
            owner.session_refresh_due,
            access_mode_change_due,
        ]
        .into_iter()
        .flatten()
        .min();
        match command_rx.try_recv() {
            Ok(RuntimeCommand::Shutdown) => running = false,
            Ok(command) => owner.apply_command(command),
            Err(mpsc::TryRecvError::Empty) => match next_deadline {
                Some(deadline) => thread::park_timeout(deadline.saturating_duration_since(now)),
                None => thread::park(),
            },
            Err(mpsc::TryRecvError::Disconnected) => running = false,
        }
    }
    if let Some(mut process) = owner.process.take() {
        let _ = process.close();
    }
    let _ = owner.event_tx.send(RuntimeEvent::Stopped);
}
