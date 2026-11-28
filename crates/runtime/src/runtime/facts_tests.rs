use super::*;

#[test]
fn goal_updates_and_explicit_clears_survive_without_a_settled_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let state = SharedStateStore::open_at(&directory.path().join("state.sqlite3")).unwrap();
    let (mut owner, _) = crate::runtime::tests::owner_without_process(directory.path().to_owned());
    owner.state = Some(state.clone());
    owner.active_session = Some(path.clone());
    owner.snapshot.selected_session = Some(path.clone());
    for goal in [
        json!({"objective":"Finish UI","status":"active","tokenBudget":1000,"tokensUsed":200}),
        Value::Null,
    ] {
        owner.apply_process_item(SessionEvent::Activity(
            json!({"type":"session_goal_changed","goal":goal}).into(),
        ));
        let saved = state
            .with(|store| store.load_session_facts(Backend::Pi, &path))
            .unwrap();
        assert_eq!(
            serde_json::to_value(saved.goal).unwrap(),
            serde_json::to_value(owner.snapshot.session_goal.clone()).unwrap()
        );
        assert_eq!(owner.snapshot.session_goal.is_none(), goal.is_null());
    }
}

#[test]
fn terminal_outcomes_survive_preview_while_active_work_takes_priority() {
    for (outcome, expected) in [
        (SessionOutcome::Done, RunStatus::Done),
        (SessionOutcome::Failed, RunStatus::Failed),
        (SessionOutcome::Stopped, RunStatus::Stopped),
    ] {
        let mut snapshot = RuntimeSnapshot {
            history_preview: true,
            selected_session: Some("/session".into()),
            last_outcome: Some(outcome),
            ..Default::default()
        };
        assert_eq!(semantic_status(&snapshot), expected);
        conversation_mut(&mut snapshot).running = true;
        assert_eq!(semantic_status(&snapshot), RunStatus::Working);
    }
}

struct AcceptCommands;

impl SessionTransport for AcceptCommands {
    fn send(&mut self, _: SessionCommand) -> Result<String, String> {
        Ok("refresh".into())
    }
    fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<SessionEvent> {
        None
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn run_outcomes_persist_and_starting_a_turn_keeps_the_previous_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let state = SharedStateStore::open_at(&directory.path().join("state.sqlite3")).unwrap();
    let (mut owner, _) = crate::runtime::tests::owner_without_process(directory.path().to_owned());
    owner.state = Some(state.clone());
    owner.process = Some(Box::new(AcceptCommands));
    owner.active_session = Some(path.clone());
    owner.snapshot.selected_session = Some(path.clone());
    let read = || {
        state
            .with(|store| store.load_session_facts(Backend::Pi, &path))
            .unwrap()
            .outcome
    };
    for (reason, expected) in [
        ("error", SessionOutcome::Failed),
        ("aborted", SessionOutcome::Stopped),
        ("stop", SessionOutcome::Done),
    ] {
        let previous = read();
        owner.apply_process_item(SessionEvent::Activity(json!({"type":"agent_start"}).into()));
        assert_eq!(
            read(),
            previous,
            "starting a turn must retain the last outcome"
        );
        assert_eq!(semantic_status(&owner.snapshot), RunStatus::Working);
        owner.apply_process_item(SessionEvent::Activity(
            json!({"type":"message_end","message":{
                "role":"assistant","content":[],"stopReason":reason,"errorMessage":"failed"
            }})
            .into(),
        ));
        owner.apply_process_item(SessionEvent::Activity(
            json!({"type":"agent_settled"}).into(),
        ));
        assert_eq!(read(), Some(expected));
    }
    owner.apply_process_item(SessionEvent::Activity(json!({"type":"agent_start"}).into()));
    owner.apply_response(agents::SessionResponse::success(
        None,
        agents::SessionResponsePayload::Abort,
    ));
    assert_eq!(
        read(),
        Some(SessionOutcome::Done),
        "abort acknowledgement is not settlement"
    );
    owner.apply_process_item(SessionEvent::Activity(
        json!({"type":"agent_settled"}).into(),
    ));
    assert_eq!(read(), Some(SessionOutcome::Stopped));
    owner.start_process(Some(path.clone()));
    assert_eq!(owner.snapshot.status, "Failed");
    assert!(owner.process.is_none());
    assert_eq!(read(), Some(SessionOutcome::Stopped));
    owner.apply_process_item(SessionEvent::Activity(json!({"type":"agent_start"}).into()));
    owner.fail("transport closed during a run".into());
    assert_eq!(read(), Some(SessionOutcome::Failed));
}

#[test]
fn unavailable_access_mode_keeps_the_reload_target() {
    let (mut owner, _) = crate::runtime::tests::owner_without_process("/project".into());
    let path = PathBuf::from("/project/resume.jsonl");
    owner.harness = None;
    owner.start_process(Some(path.clone()));
    assert!(
        owner
            .snapshot
            .conversation
            .diagnostics
            .iter()
            .any(|s| s.contains("No access mode"))
    );
    owner.harness = Some(Backend::Pi);
    owner.reload();
    assert_eq!(owner.active_session.as_ref(), Some(&path));
    assert_eq!(owner.snapshot.selected_session.as_ref(), Some(&path));
}

fn apply_session_state(owner: &mut RuntimeOwner, streaming: bool, tier: Option<&str>) {
    let state = serde_json::from_value(json!({
        "sessionId": "session",
        "isStreaming": streaming,
        "isCompacting": false,
        "serviceTier": tier,
        "autoCompactionEnabled": true,
        "messageCount": 1,
        "pendingMessageCount": 0,
    }))
    .unwrap();
    owner.apply_response(agents::SessionResponse::success(
        None,
        agents::SessionResponsePayload::LoadState(Box::new(state)),
    ));
}

#[test]
fn disconnect_uses_only_activity_that_is_still_in_progress() {
    for (events, status, outcome) in [
        (
            vec!["agent_start"],
            RunStatus::Working,
            SessionOutcome::Failed,
        ),
        (
            vec!["compaction_start"],
            RunStatus::Compacting,
            SessionOutcome::Failed,
        ),
        (
            vec!["compaction_start", "compaction_end"],
            RunStatus::Done,
            SessionOutcome::Done,
        ),
        (
            vec!["agent_start", "compaction_start", "compaction_end"],
            RunStatus::Working,
            SessionOutcome::Failed,
        ),
        (vec![], RunStatus::Done, SessionOutcome::Done),
    ] {
        let (mut owner, _) = crate::runtime::tests::owner_without_process("/project".into());
        owner.process = Some(Box::new(AcceptCommands));
        owner.snapshot.last_outcome = Some(SessionOutcome::Done);
        if events.is_empty() {
            apply_session_state(&mut owner, true, None);
            assert_eq!(semantic_status(&owner.snapshot), RunStatus::Working);
        }
        for event in &events {
            owner.apply_process_item(SessionEvent::Activity(json!({"type":event}).into()));
        }
        apply_session_state(&mut owner, false, None);
        assert_eq!(semantic_status(&owner.snapshot), status, "{events:?}");
        owner.apply_process_item(SessionEvent::Failure("EOF".into()));
        assert_eq!(owner.snapshot.last_outcome, Some(outcome), "{events:?}");
    }
}

#[test]
fn tier_observations_follow_backend_reports_while_selection_is_pending() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let state = SharedStateStore::open_at(&directory.path().join("state.sqlite3")).unwrap();
    let (mut owner, _) = crate::runtime::tests::owner_without_process(directory.path().to_owned());
    owner.state = Some(state.clone());
    owner.active_session = Some(path.clone());
    owner.snapshot.selected_session = Some(path.clone());
    owner.snapshot.restore_session_facts(SessionFacts {
        service_tier: Some("priority".into()),
        ..Default::default()
    });
    assert_eq!(owner.snapshot.selected_service_tier(), Some("priority"));
    owner.snapshot.prefill_service_tier = Some("fast".into());
    owner.snapshot.pending_initial_service_tier = true;
    owner.publish_session_metadata();
    let observed = || {
        state
            .with(|store| store.load_session_facts(Backend::Pi, &path))
            .unwrap()
            .service_tier
    };
    assert_eq!(owner.snapshot.selected_service_tier(), Some("fast"));
    assert_eq!(observed().as_deref(), Some("priority"));
    for tier in [None, Some("fast")] {
        apply_session_state(&mut owner, false, tier);
        assert_eq!(owner.snapshot.selected_service_tier(), Some("fast"));
        assert_eq!(observed().as_deref(), tier);
    }
}
