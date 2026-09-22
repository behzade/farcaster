use super::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Wire {
    commands: Vec<SessionCommand>,
    events: VecDeque<SessionEvent>,
    reject_sends: HashSet<usize>,
    send_errors: HashMap<usize, String>,
}
struct Transport(Arc<Mutex<Wire>>, bool);
impl SessionTransport for Transport {
    fn tracks_prompt_delivery(&self, _: PromptMode) -> bool {
        self.1
    }
    fn send(&mut self, command: SessionCommand) -> Result<String, String> {
        let mut wire = self.0.lock().expect("wire");
        wire.commands.push(command);
        if wire.reject_sends.contains(&wire.commands.len()) {
            return Err("rejected before write".into());
        }
        if let Some(error) = wire.send_errors.get(&wire.commands.len()) {
            return Err(error.clone());
        }
        Ok(format!("native-{}", wire.commands.len()))
    }
    fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<SessionEvent> {
        self.0.lock().expect("wire").events.pop_front()
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}
fn session(policy: SteeringBoundary, tracked: bool) -> (QueuedSession, Arc<Mutex<Wire>>) {
    let wire = Arc::new(Mutex::new(Wire::default()));
    let mut session = QueuedSession::new(Box::new(Transport(wire.clone(), tracked)), policy, None);
    session.session_id = Some("parent".into());
    session.running = true;
    (session, wire)
}
fn enqueue(session: &mut QueuedSession, mode: PromptMode) -> String {
    session
        .send(SessionCommand::Prompt {
            mode,
            message: "same text".into(),
            images: vec![],
        })
        .expect("enqueue")
}
fn drain(session: &mut QueuedSession) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    for _ in 0..100 {
        let Some(event) = session.poll() else { break };
        events.push(event);
    }
    events
}
fn boundary() -> (PromptBoundary, Boundary, std::thread::JoinHandle<String>) {
    let gate = PromptBoundary::new(None).expect("gate");
    gate.enable(true);
    let response = super::super::prompt_boundary::tests::request(&gate.url, "parent");
    let boundary = super::super::prompt_boundary::tests::receive(&gate);
    (gate, boundary, response)
}

#[test]
fn followups_stay_local_and_exact_id_cancellation_does_not_touch_backend() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let first = enqueue(&mut session, PromptMode::FollowUp);
    let second = enqueue(&mut session, PromptMode::FollowUp);
    drain(&mut session);
    assert!(wire.lock().expect("wire").commands.is_empty());
    session.cancel_prompt(&first).expect("cancel first");
    assert_eq!(session.queue.len(), 1);
    assert_eq!(session.queue[0].id, second);
    assert!(wire.lock().expect("wire").commands.is_empty());
    wire.lock()
        .expect("wire")
        .events
        .push_back(activity(json!({"type":"agent_settled"})));
    drain(&mut session);
    assert!(matches!(
        &wire.lock().expect("wire").commands[0],
        SessionCommand::Prompt {
            mode: PromptMode::Normal,
            ..
        }
    ));
}

#[test]
fn native_steer_turn_race_becomes_a_next_turn_without_a_prompt_error() {
    let (mut session, wire) = session(SteeringBoundary::Native, true);
    wire.lock()
        .expect("wire")
        .send_errors
        .insert(1, "Codex worker has not reported its active turn".into());
    let id = enqueue(&mut session, PromptMode::Steer);
    assert_eq!(session.queue.len(), 1);
    assert_eq!(session.queue[0].id, id);
    let events = drain(&mut session);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SessionEvent::Response(_)))
    );
    let commands = &wire.lock().expect("wire").commands;
    assert!(matches!(
        commands[1],
        SessionCommand::Prompt {
            mode: PromptMode::Normal,
            ..
        }
    ));
    assert_eq!(session.dispatched["native-2"].inputs[0].id, id);
}

#[test]
fn only_locally_owned_queue_rows_advertise_cancellation() {
    let (mut session, wire) = session(SteeringBoundary::Native, true);
    let steer = enqueue(&mut session, PromptMode::Steer);
    let followup = enqueue(&mut session, PromptMode::FollowUp);
    // Native IDs identify inputs; they do not establish local ownership.
    wire.lock().expect("wire").events.push_back(activity(json!({
        "type":"queue_update", "steering":["same text"], "steeringIds":[steer],
        "cancellableIds":["untrusted-native-cancellation"]
    })));
    let events = drain(&mut session);
    let queue = events
        .iter()
        .rev()
        .find_map(|event| match event {
            SessionEvent::Activity(body) if body.value()["type"] == "queue_update" => {
                Some(body.value())
            }
            _ => None,
        })
        .expect("queue update");
    assert_eq!(queue["cancellableIds"], json!([followup]));
    session
        .cancel_prompt(&steer)
        .expect("stale native row click must be harmless");
    assert!(!drain(&mut session).iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["type"] == "prompt_delivery")));
    assert_eq!(wire.lock().expect("wire").commands.len(), 1);
}

#[test]
fn stale_cancel_after_dispatch_does_not_cancel_or_reject_delivery() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let id = enqueue(&mut session, PromptMode::Steer);
    session.running = false;
    drain(&mut session);
    session.cancel_prompt(&id).expect("delivery won the race");
    session.cancel_prompt(&id).expect("repeated stale click");
    assert!(
        drain(&mut session).is_empty(),
        "no fabricated cancellation or rejection"
    );
    wire.lock().expect("wire").events.extend([
        activity(json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"delivered", "message":{}})),
        SessionEvent::Response(SessionResponse::success(Some("native-1".into()), Payload::Prompt(PromptMode::Normal))),
    ]);
    let events = drain(&mut session);
    assert!(events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["submissionId"] == id && body.value()["status"] == "delivered")));
    session.cancel_prompt(&id).expect("already delivered click");
    assert!(drain(&mut session).is_empty());
    assert_eq!(wire.lock().expect("wire").commands.len(), 1);
}

#[test]
fn repeated_cancel_of_a_local_row_emits_one_cancellation() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let id = enqueue(&mut session, PromptMode::FollowUp);
    drain(&mut session);
    session.cancel_prompt(&id).expect("cancel");
    session.cancel_prompt(&id).expect("second queued click");
    let events = drain(&mut session);
    assert_eq!(events.iter().filter(|event| matches!(event, SessionEvent::Activity(body) if body.value()["status"] == "cancelled" && body.value()["submissionId"] == id)).count(), 1);
    assert!(wire.lock().expect("wire").commands.is_empty());
}

#[test]
fn held_hook_selects_steer_past_followup_and_waits_for_admission() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::FollowUp);
    let steer = enqueue(&mut session, PromptMode::Steer);
    let (_gate, boundary, response) = boundary();
    session.boundary(boundary);
    drain(&mut session);
    assert_eq!(session.queue.len(), 1);
    assert!(!response.is_finished());
    assert!(matches!(
        &wire.lock().expect("wire").commands[0],
        SessionCommand::Prompt {
            mode: PromptMode::Steer,
            ..
        }
    ));
    session
        .cancel_prompt(&steer)
        .expect("claimed row click is ignored");
    wire.lock().expect("wire").events.push_back(activity(json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"accepted", "message":{}})));
    let events = drain(&mut session);
    assert!(events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["submissionId"] == steer)));
    assert!(response.join().expect("hook thread").ends_with("{}"));
}

#[test]
fn child_session_hook_cannot_claim_parent_input() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::Steer);
    let (_gate, mut boundary, response) = boundary();
    boundary.session_id = "child".into();
    session.boundary(boundary);
    assert_eq!(session.queue.len(), 1);
    assert!(wire.lock().expect("wire").commands.is_empty());
    assert!(response.join().expect("hook thread").ends_with("{}"));
}

#[test]
fn held_boundary_admits_every_pending_steer_before_releasing_the_batch() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let first = enqueue(&mut session, PromptMode::Steer);
    let followup = enqueue(&mut session, PromptMode::FollowUp);
    let second = enqueue(&mut session, PromptMode::Steer);
    let third = enqueue(&mut session, PromptMode::Steer);
    let (_gate, hook, response) = boundary();
    session.boundary(hook);
    drain(&mut session);
    assert_eq!(session.queue.len(), 1);
    assert_eq!(session.queue[0].id, followup);
    assert_eq!(wire.lock().expect("wire").commands.len(), 3);
    for (native, input) in [
        ("native-1", &first),
        ("native-2", &second),
        ("native-3", &third),
    ] {
        assert_eq!(session.dispatched[native].inputs[0].id, *input);
        session
            .cancel_prompt(input)
            .expect("claimed row click is ignored");
    }
    // A later steer belongs to the next opportunity, not this snapshot.
    let later = enqueue(&mut session, PromptMode::Steer);
    // Out-of-order and duplicate admission evidence must not release early.
    for native in ["native-2", "native-2", "native-1"] {
        session.observe(activity(json!({"type":"prompt_delivery", "submissionId":native, "status":"accepted", "message":{}})));
        assert!(session.held_batch.is_some());
        assert!(!response.is_finished());
    }
    session.observe(SessionEvent::Response(SessionResponse::success(
        Some("native-3".into()),
        Payload::Prompt(PromptMode::Steer),
    )));
    assert!(session.held_batch.is_none());
    assert!(response.join().expect("hook thread").ends_with("{}"));
    assert_eq!(
        session
            .queue
            .iter()
            .map(|input| input.id.as_str())
            .collect::<Vec<_>>(),
        [followup.as_str(), later.as_str()]
    );
}

#[test]
fn rejected_batch_member_does_not_release_before_other_members_or_block_forever() {
    let (mut session, wire) = session(SteeringBoundary::Held, false);
    wire.lock().expect("wire").reject_sends.insert(2);
    for _ in 0..3 {
        enqueue(&mut session, PromptMode::Steer);
    }
    let (_gate, hook, response) = boundary();
    session.boundary(hook);
    assert_eq!(session.held_batch.as_ref().expect("batch").pending.len(), 2);
    session.observe(SessionEvent::Response(SessionResponse::success(
        Some("native-3".into()),
        Payload::Prompt(PromptMode::Steer),
    )));
    assert!(session.held_batch.is_some());
    session.observe(SessionEvent::Response(SessionResponse::failure(
        Some("native-1".into()),
        SessionOperation::Prompt(PromptMode::Steer),
        "rejected".into(),
    )));
    assert!(session.held_batch.is_none());
    assert!(response.join().expect("hook thread").ends_with("{}"));
}

#[test]
fn closing_during_batch_admission_releases_the_hook() {
    let (mut session, _) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::Steer);
    enqueue(&mut session, PromptMode::Steer);
    let (_gate, hook, response) = boundary();
    session.boundary(hook);
    session.close().expect("close");
    assert!(response.join().expect("hook thread").ends_with("{}"));
}

#[test]
fn stop_after_batch_dispatches_only_after_settlement_without_abort() {
    let (mut session, wire) = session(SteeringBoundary::StopAfterBatch, true);
    let steer = enqueue(&mut session, PromptMode::Steer);
    let (_gate, boundary, response) = boundary();
    session.boundary(boundary);
    assert!(
        response
            .join()
            .expect("hook thread")
            .contains("\"continue\":false")
    );
    drain(&mut session);
    assert!(wire.lock().expect("wire").commands.is_empty());
    session
        .cancel_prompt(&steer)
        .expect("claimed row click is ignored");
    wire.lock()
        .expect("wire")
        .events
        .push_back(activity(json!({"type":"agent_settled"})));
    drain(&mut session);
    assert!(matches!(
        &wire.lock().expect("wire").commands[..],
        [SessionCommand::Prompt {
            mode: PromptMode::Normal,
            ..
        }]
    ));
}

#[test]
fn receipt_before_response_keeps_identity_then_releases_mapping() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let id = enqueue(&mut session, PromptMode::FollowUp);
    session.running = false;
    drain(&mut session);
    wire.lock().expect("wire").events.extend([
        activity(json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"delivered", "message":{}})),
        SessionEvent::Response(SessionResponse::success(Some("native-1".into()), Payload::Prompt(PromptMode::Normal))),
    ]);
    let events = drain(&mut session);
    assert!(events.iter().any(|event| matches!(event, SessionEvent::Response(response) if response.id.as_deref() == Some(&id) && response.operation() == SessionOperation::Prompt(PromptMode::FollowUp))));
    assert!(session.dispatched.is_empty());
}

#[test]
fn untracked_backend_ack_is_not_reported_as_delivery() {
    let (mut session, wire) = session(SteeringBoundary::Held, false);
    enqueue(&mut session, PromptMode::FollowUp);
    session.running = false;
    drain(&mut session);
    wire.lock()
        .expect("wire")
        .events
        .push_back(SessionEvent::Response(SessionResponse::success(
            Some("native-1".into()),
            Payload::Prompt(PromptMode::Normal),
        )));
    let events = drain(&mut session);
    assert!(!events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["status"] == "delivered")));
    assert!(!session.tracks_prompt_delivery(PromptMode::FollowUp));
}

#[test]
fn unsupported_steer_remains_followup_but_reply_preserves_requested_operation() {
    let (mut session, wire) = session(SteeringBoundary::Unsupported, false);
    let id = enqueue(&mut session, PromptMode::Steer);
    drain(&mut session);
    assert_eq!(session.queue[0].mode, PromptMode::FollowUp);
    assert!(wire.lock().expect("wire").commands.is_empty());
    session.running = false;
    drain(&mut session);
    wire.lock()
        .expect("wire")
        .events
        .push_back(SessionEvent::Response(SessionResponse::success(
            Some("native-1".into()),
            Payload::Prompt(PromptMode::Normal),
        )));
    assert!(drain(&mut session).iter().any(|event| matches!(event, SessionEvent::Response(response) if response.id.as_deref() == Some(&id) && response.operation() == SessionOperation::Prompt(PromptMode::Steer))));
}

#[test]
fn apply_with_an_empty_local_queue_reaches_native_pending_inputs() {
    for policy in [
        SteeringBoundary::Held,
        SteeringBoundary::StopAfterBatch,
        SteeringBoundary::Native,
        SteeringBoundary::Unsupported,
    ] {
        let (mut session, wire) = session(policy, true);
        session.send(SessionCommand::ApplySteering).expect("apply");
        drain(&mut session);
        assert!(matches!(
            &wire.lock().expect("wire").commands[..],
            [SessionCommand::ApplySteering]
        ));
    }
}

#[test]
fn rejected_idle_dispatch_does_not_block_the_next_followup() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::FollowUp);
    enqueue(&mut session, PromptMode::FollowUp);
    session.running = false;
    drain(&mut session);
    wire.lock()
        .expect("wire")
        .events
        .push_back(SessionEvent::Response(SessionResponse::failure(
            Some("native-1".into()),
            SessionOperation::Prompt(PromptMode::Normal),
            "rejected".into(),
        )));
    drain(&mut session);
    assert_eq!(wire.lock().expect("wire").commands.len(), 2);
}

#[test]
fn clearing_queue_cancels_only_unclaimed_input() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let claimed = enqueue(&mut session, PromptMode::FollowUp);
    let unclaimed = enqueue(&mut session, PromptMode::FollowUp);
    session.running = false;
    drain(&mut session);
    session.clear_queue().expect("clear local queue");
    let events = drain(&mut session);
    let cancelled = events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Activity(body) if body.value()["status"] == "cancelled" => {
                body.value()["submissionId"].as_str()
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cancelled, [unclaimed]);
    assert!(
        session
            .dispatched
            .values()
            .any(|dispatch| dispatch.inputs.iter().any(|input| input.id == claimed))
    );
    assert_eq!(wire.lock().expect("wire").commands.len(), 1);
}

#[test]
fn native_compatibility_stages_followup_before_immediate_apply() {
    let (mut session, wire) = session(SteeringBoundary::Native, true);
    enqueue(&mut session, PromptMode::FollowUp);
    session
        .send(SessionCommand::ApplySteering)
        .expect("promote");
    drain(&mut session);
    assert!(session.queue.is_empty());
    assert!(matches!(
        &wire.lock().expect("wire").commands[..],
        [
            SessionCommand::Prompt {
                mode: PromptMode::FollowUp,
                ..
            },
            SessionCommand::ApplySteering
        ]
    ));
}

#[test]
fn stale_idle_state_does_not_dispatch_while_a_prompt_is_starting() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::Normal);
    enqueue(&mut session, PromptMode::FollowUp);
    let state = serde_json::from_value(json!({"sessionId":"parent", "isStreaming":false,
        "isCompacting":false, "autoCompactionEnabled":false, "messageCount":0, "pendingMessageCount":0})).expect("state");
    wire.lock()
        .expect("wire")
        .events
        .push_back(SessionEvent::Response(SessionResponse::success(
            None,
            Payload::LoadState(Box::new(state)),
        )));
    drain(&mut session);
    assert_eq!(session.queue.len(), 1);
    assert_eq!(wire.lock().expect("wire").commands.len(), 1);
}

#[test]
fn escape_stages_all_local_inputs_before_native_handoff_on_every_backend() {
    for policy in [
        SteeringBoundary::Held,
        SteeringBoundary::StopAfterBatch,
        SteeringBoundary::Native,
        SteeringBoundary::Unsupported,
    ] {
        let (mut session, wire) = session(policy, true);
        let first = enqueue(&mut session, PromptMode::FollowUp);
        let second = enqueue(&mut session, PromptMode::FollowUp);
        drain(&mut session);
        assert!(wire.lock().expect("wire").commands.is_empty());
        let apply = session.send(SessionCommand::ApplySteering).expect("apply");
        assert_eq!(apply, "native-3");
        assert_eq!(session.dispatched["native-1"].inputs[0].id, first);
        assert_eq!(session.dispatched["native-2"].inputs[0].id, second);
        assert!(matches!(
            &wire.lock().expect("wire").commands[..],
            [
                SessionCommand::Prompt {
                    mode: PromptMode::FollowUp,
                    ..
                },
                SessionCommand::Prompt {
                    mode: PromptMode::FollowUp,
                    ..
                },
                SessionCommand::ApplySteering,
            ]
        ));
        // The wrapper must not invent a successful control response.
        assert!(
            !drain(&mut session)
                .iter()
                .any(|event| matches!(event, SessionEvent::Response(_)))
        );
        wire.lock()
            .expect("wire")
            .events
            .push_back(SessionEvent::Response(SessionResponse::success(
                Some(apply.clone()),
                Payload::ApplySteering,
            )));
        assert!(drain(&mut session).iter().any(|event| matches!(event, SessionEvent::Response(response) if response.id.as_deref() == Some(&apply))));
    }
}

#[test]
fn escape_control_failure_is_not_reported_as_success_or_requeued() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::Steer);
    wire.lock().expect("wire").reject_sends.insert(2);
    assert!(session.send(SessionCommand::ApplySteering).is_err());
    assert!(session.queue.is_empty());
    assert_eq!(session.dispatched.len(), 1);
    assert!(!drain(&mut session).iter().any(|event| matches!(event, SessionEvent::Response(response) if response.operation() == SessionOperation::ApplySteering)));
}

#[test]
fn idle_escape_fences_later_followups_until_the_handoff_settles() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    session.running = false;
    enqueue(&mut session, PromptMode::FollowUp);
    session
        .send(SessionCommand::ApplySteering)
        .expect("apply while idle");
    enqueue(&mut session, PromptMode::FollowUp);
    drain(&mut session);
    assert_eq!(wire.lock().expect("wire").commands.len(), 2);
    assert_eq!(session.queue.len(), 1);
}

#[test]
fn second_escape_reaches_native_abort_and_does_not_resubmit_handoff_inputs() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let first = enqueue(&mut session, PromptMode::Steer);
    session.send(SessionCommand::ApplySteering).expect("apply");
    let later = enqueue(&mut session, PromptMode::FollowUp);
    session.send(SessionCommand::Abort).expect("abort");
    wire.lock()
        .expect("wire")
        .events
        .push_back(activity(json!({"type":"agent_settled"})));
    let events = drain(&mut session);
    assert!(events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["submissionId"] == later && body.value()["status"] == "cancelled")));
    assert_eq!(session.dispatched["native-1"].inputs[0].id, first);
    assert!(matches!(
        &wire.lock().expect("wire").commands[..],
        [
            SessionCommand::Prompt { .. },
            SessionCommand::ApplySteering,
            SessionCommand::Abort
        ]
    ));
}

#[test]
fn escape_releases_held_hook_without_resubmitting_claimed_steers() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::Steer);
    let (_gate, hook, response) = boundary();
    session.boundary(hook);
    assert!(session.held_batch.is_some());
    session.send(SessionCommand::ApplySteering).expect("apply");
    assert!(response.join().expect("hook thread").ends_with("{}"));
    assert!(session.held_batch.is_none());
    assert!(matches!(
        &wire.lock().expect("wire").commands[..],
        [SessionCommand::Prompt { .. }, SessionCommand::ApplySteering]
    ));
}

#[test]
fn unknown_response_retains_identity_for_late_delivery() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let id = enqueue(&mut session, PromptMode::FollowUp);
    session.running = false;
    drain(&mut session);
    wire.lock().expect("wire").events.extend([
        SessionEvent::Response(SessionResponse::prompt_delivery_unknown("native-1".into(), PromptMode::Normal, "unknown".into())),
        activity(json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"delivered", "message":{}})),
    ]);
    let events = drain(&mut session);
    assert!(events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["status"] == "delivered" && body.value()["submissionId"] == id)));
    assert!(session.dispatched.is_empty());
}

#[test]
fn transport_failure_does_not_report_unsent_input_as_cancelled() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    enqueue(&mut session, PromptMode::FollowUp);
    wire.lock()
        .expect("wire")
        .events
        .push_back(SessionEvent::Failure("lost connection".into()));
    let events = drain(&mut session);
    assert_eq!(session.queue.len(), 1);
    assert!(!events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["status"] == "cancelled")));
}

#[test]
fn text_only_settlement_sends_all_pending_steers_in_one_native_prompt() {
    for policy in [SteeringBoundary::Held, SteeringBoundary::StopAfterBatch] {
        let (mut session, wire) = session(policy, true);
        let followup = enqueue(&mut session, PromptMode::FollowUp);
        let mut ids = Vec::new();
        for text in [
            "first instruction",
            "second instruction",
            "third instruction",
        ] {
            ids.push(
                session
                    .send(SessionCommand::Prompt {
                        mode: PromptMode::Steer,
                        message: text.into(),
                        images: vec![],
                    })
                    .expect("enqueue steer"),
            );
        }
        drain(&mut session);
        assert!(wire.lock().expect("wire").commands.is_empty());
        wire.lock()
            .expect("wire")
            .events
            .push_back(activity(json!({"type":"agent_settled"})));
        drain(&mut session);
        {
            let wire = wire.lock().expect("wire");
            let [
                SessionCommand::Prompt {
                    mode: PromptMode::Normal,
                    message,
                    ..
                },
            ] = &wire.commands[..]
            else {
                panic!("the next turn must start with one complete batch");
            };
            assert_eq!(
                message,
                "first instruction\n\nsecond instruction\n\nthird instruction"
            );
        }
        assert_eq!(session.queue.len(), 1);
        assert_eq!(session.queue[0].id, followup);
        // One native receipt proves delivery of the entire submitted envelope,
        // not just its first member. Each logical input keeps its own reply.
        wire.lock().expect("wire").events.extend([
            activity(json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"delivered", "message":{}})),
            SessionEvent::Response(SessionResponse::success(Some("native-1".into()), Payload::Prompt(PromptMode::Normal))),
        ]);
        let events = drain(&mut session);
        for id in &ids {
            assert_eq!(events.iter().filter(|event| matches!(event, SessionEvent::Activity(body) if body.value()["submissionId"] == *id && body.value()["status"] == "delivered")).count(), 1);
            assert_eq!(events.iter().filter(|event| matches!(event, SessionEvent::Response(response) if response.id.as_deref() == Some(id) && response.operation() == SessionOperation::Prompt(PromptMode::Steer))).count(), 1);
        }
        assert_eq!(
            wire.lock().expect("wire").commands.len(),
            1,
            "follow-ups still wait for settlement"
        );
    }
}

#[test]
fn idle_batch_keeps_duplicate_inputs_images_and_native_receipt_evidence() {
    for receipt_first in [false, true] {
        let (mut session, wire) = session(SteeringBoundary::Held, true);
        let images = [
            PromptImage::new("YQ==".into(), "image/png".into()),
            PromptImage::new("Yg==".into(), "image/jpeg".into()),
        ];
        let ids = images
            .iter()
            .map(|image| {
                session
                    .send(SessionCommand::Prompt {
                        mode: PromptMode::Steer,
                        message: "same text".into(),
                        images: vec![image.clone()],
                    })
                    .expect("enqueue")
            })
            .collect::<Vec<_>>();
        session.running = false;
        let before_receipt = drain(&mut session);
        assert!(
            !before_receipt
                .iter()
                .any(|event| matches!(event, SessionEvent::Response(_)))
        );
        assert!(!before_receipt.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["type"] == "prompt_delivery")));
        let wire_guard = wire.lock().expect("wire");
        let SessionCommand::Prompt {
            message,
            images: sent,
            ..
        } = &wire_guard.commands[0]
        else {
            panic!("prompt")
        };
        assert_eq!(message, "same text\n\nsame text");
        assert_eq!(sent, &images);
        drop(wire_guard);
        for id in &ids {
            session
                .cancel_prompt(id)
                .expect("claimed row click is ignored");
        }
        let response = SessionEvent::Response(SessionResponse::success(
            Some("native-1".into()),
            Payload::Prompt(PromptMode::Normal),
        ));
        let receipt = activity(
            json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"delivered", "message":{"content":[{"type":"text","text":"same text\n\nsame text"}]}}),
        );
        let (first, second) = if receipt_first {
            (receipt, response)
        } else {
            (response, receipt)
        };
        wire.lock().expect("wire").events.push_back(first);
        let mut events = drain(&mut session);
        assert_eq!(
            session.dispatched.len(),
            1,
            "retain mapping until both reply and delivery"
        );
        wire.lock().expect("wire").events.push_back(second);
        events.extend(drain(&mut session));
        assert!(session.dispatched.is_empty());
        wire.lock().expect("wire").events.push_back(activity(json!({
            "type":"prompt_delivery", "submissionId":"native-1",
            "status":"delivered", "message":{"content":[{"type":"text","text":"same text\n\nsame text"}]}
        })));
        assert!(
            drain(&mut session).is_empty(),
            "a repeated native receipt must not leak the batch into the transcript"
        );
        for (id, image) in ids.iter().zip(images) {
            let receipts = events
                .iter()
                .filter_map(|event| match event {
                    SessionEvent::Activity(body) if body.value()["submissionId"] == *id => {
                        Some(body.value())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(receipts.len(), 1);
            assert_eq!(receipts[0]["status"], "delivered");
            assert_eq!(
                receipts[0]["message"]["content"],
                json!([
                    {"type":"text", "text":"same text"},
                    {"type":"image", "data":image.data, "mimeType":image.mime_type}
                ])
            );
        }
    }
}

#[test]
fn idle_batch_rejection_resolves_every_member_and_allows_the_next_followup() {
    for synchronous in [false, true] {
        let (mut session, wire) = session(SteeringBoundary::Held, true);
        let ids = [
            enqueue(&mut session, PromptMode::Steer),
            enqueue(&mut session, PromptMode::Steer),
        ];
        let followup = enqueue(&mut session, PromptMode::FollowUp);
        if synchronous {
            wire.lock().expect("wire").reject_sends.insert(1);
        }
        session.running = false;
        let mut events = drain(&mut session);
        if !synchronous {
            wire.lock()
                .expect("wire")
                .events
                .push_back(SessionEvent::Response(SessionResponse::failure(
                    Some("native-1".into()),
                    SessionOperation::Prompt(PromptMode::Normal),
                    "rejected".into(),
                )));
            events.extend(drain(&mut session));
        }
        for id in ids {
            let replies = events
                .iter()
                .filter_map(|event| match event {
                    SessionEvent::Response(response) if response.id.as_deref() == Some(&id) => {
                        Some(response)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(replies.len(), 1);
            assert_eq!(
                replies[0].operation(),
                SessionOperation::Prompt(PromptMode::Steer)
            );
            assert_eq!(
                replies[0].result.as_ref().expect_err("rejection").kind,
                crate::SessionResponseErrorKind::RejectedBeforeAcceptance
            );
        }
        assert_eq!(wire.lock().expect("wire").commands.len(), 2);
        assert_eq!(session.dispatched["native-2"].inputs[0].id, followup);
    }
}

#[test]
fn idle_batch_unknown_reply_keeps_each_identity_for_late_delivery() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let ids = [
        enqueue(&mut session, PromptMode::Steer),
        enqueue(&mut session, PromptMode::Steer),
    ];
    session.running = false;
    drain(&mut session);
    wire.lock()
        .expect("wire")
        .events
        .push_back(SessionEvent::Response(
            SessionResponse::prompt_delivery_unknown(
                "native-1".into(),
                PromptMode::Normal,
                "connection lost".into(),
            ),
        ));
    let events = drain(&mut session);
    for id in &ids {
        assert!(events.iter().any(|event| matches!(event, SessionEvent::Response(response) if response.id.as_deref() == Some(id) && response.result.as_ref().is_err_and(|error| error.kind == crate::SessionResponseErrorKind::DeliveryUnknown))));
    }
    assert_eq!(session.dispatched.len(), 1);
    wire.lock().expect("wire").events.push_back(activity(json!({"type":"prompt_delivery", "submissionId":"native-1", "status":"delivered", "message":{}})));
    let events = drain(&mut session);
    for id in ids {
        assert!(events.iter().any(|event| matches!(event, SessionEvent::Activity(body) if body.value()["submissionId"] == id && body.value()["status"] == "delivered")));
    }
    assert!(session.dispatched.is_empty());
}

#[test]
fn stopped_boundary_claims_a_snapshot_and_leaves_later_steers_cancellable() {
    let (mut session, wire) = session(SteeringBoundary::StopAfterBatch, true);
    let first = enqueue(&mut session, PromptMode::Steer);
    let second = enqueue(&mut session, PromptMode::Steer);
    let (_gate, hook, response) = boundary();
    session.boundary(hook);
    assert!(
        response
            .join()
            .expect("hook")
            .contains("\"continue\":false")
    );
    session
        .cancel_prompt(&first)
        .expect("claimed row click is ignored");
    session
        .cancel_prompt(&second)
        .expect("claimed row click is ignored");
    let later = enqueue(&mut session, PromptMode::Steer);
    session.cancel_prompt(&later).expect("cancel later input");
    drain(&mut session);
    assert!(wire.lock().expect("wire").commands.is_empty());
    wire.lock()
        .expect("wire")
        .events
        .push_back(activity(json!({"type":"agent_settled"})));
    drain(&mut session);
    assert_eq!(wire.lock().expect("wire").commands.len(), 1);
    assert_eq!(
        session.dispatched["native-1"]
            .inputs
            .iter()
            .map(|input| input.id.as_str())
            .collect::<Vec<_>>(),
        [first.as_str(), second.as_str()]
    );
}

#[test]
fn cancelling_one_steer_before_idle_omits_only_that_member_from_the_batch() {
    let (mut session, wire) = session(SteeringBoundary::Held, true);
    let first = enqueue(&mut session, PromptMode::Steer);
    let cancelled = enqueue(&mut session, PromptMode::Steer);
    let last = enqueue(&mut session, PromptMode::Steer);
    session.cancel_prompt(&cancelled).expect("cancel middle");
    session.running = false;
    drain(&mut session);
    assert_eq!(wire.lock().expect("wire").commands.len(), 1);
    assert_eq!(
        session.dispatched["native-1"]
            .inputs
            .iter()
            .map(|input| input.id.as_str())
            .collect::<Vec<_>>(),
        [first.as_str(), last.as_str()]
    );
}
