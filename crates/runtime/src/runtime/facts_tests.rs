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
    owner.fail("transport closed during a run".into());
    assert_eq!(read(), Some(SessionOutcome::Failed));
}

#[test]
fn backend_default_tier_replaces_the_restored_observation() {
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
    owner.apply_response(agents::SessionResponse::success(
        None,
        agents::SessionResponsePayload::LoadState(Box::new(
            serde_json::from_value(json!({
                "sessionId":"session", "isStreaming":false, "isCompacting":false,
                "autoCompactionEnabled":true, "messageCount":1, "pendingMessageCount":0,
                "serviceTier":null
            }))
            .unwrap(),
        )),
    ));
    assert_eq!(owner.snapshot.selected_service_tier(), None);
    assert_eq!(
        state
            .with(|store| store.load_session_facts(Backend::Pi, &path))
            .unwrap()
            .service_tier,
        None
    );
}
