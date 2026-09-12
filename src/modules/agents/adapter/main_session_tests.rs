use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use super::*;
use crate::agents::extensions::SessionState;

struct IdleWorker;

#[derive(Default)]
struct ControlledPromptState {
    requests: Vec<(String, WorkerSendMode, String)>,
    acks: VecDeque<(String, Result<(), String>)>,
    events: VecDeque<WorkerEvent>,
    aborts: usize,
}

struct ControlledPromptWorker(Arc<std::sync::Mutex<ControlledPromptState>>);

impl WorkerSession for ControlledPromptWorker {
    fn tracks_prompt_delivery(&self, _: WorkerSendMode) -> bool {
        true
    }

    fn send(&mut self, _: String, _: WorkerSendMode) -> Result<(), String> {
        Ok(())
    }
    fn submit_prompt(
        &mut self,
        id: String,
        text: String,
        mode: WorkerSendMode,
        _: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        self.0.lock().unwrap().requests.push((id, mode, text));
        Ok(false)
    }
    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), String>)> {
        self.0.lock().unwrap().acks.pop_front()
    }
    fn poll(&mut self) -> Option<WorkerEvent> {
        self.0.lock().unwrap().events.pop_front()
    }
    fn abort(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().aborts += 1;
        Ok(())
    }
    fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

fn project_transport(
    transport: &mut WorkerSessionTransport,
    conversation: &mut crate::app::views::transcript::conversation::ConversationState,
) -> Vec<SessionResponse> {
    let mut responses = Vec::new();
    while let Some(event) = transport.poll() {
        match event {
            SessionEvent::Activity(event) => {
                conversation.reduce(event.value());
            }
            SessionEvent::Response(response) => responses.push(response),
            SessionEvent::Failure(_) => {}
            other => panic!("unexpected event: {other:?}"),
        }
    }
    responses
}

#[test]
fn abort_before_ack_retains_unknown_then_reconciles_by_submission_id() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    for delivery_first in [false, true] {
        let backend = Arc::new(std::sync::Mutex::new(ControlledPromptState::default()));
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "race".into(),
            Box::new(ControlledPromptWorker(backend.clone())),
            MainSessionMetadata::default(),
            None,
        )
        .unwrap();
        let mut conversation = ConversationState::default();
        let id = transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::FollowUp,
                message: "duplicate text".into(),
                images: Vec::new(),
            })
            .unwrap();
        transport.send(SessionCommand::Abort).unwrap();
        let mut responses = project_transport(&mut transport, &mut conversation);
        assert_eq!(backend.lock().unwrap().aborts, 1);
        assert_eq!(backend.lock().unwrap().requests.len(), 1);
        assert_eq!(conversation.items.len(), 1);
        assert_eq!(conversation.items[0].label, "Delivery unknown");
        assert!(
            !responses
                .iter()
                .any(|r| matches!(r.operation(), SessionOperation::Prompt(_))),
            "interrupt is not a prompt acknowledgement"
        );
        let delivered = WorkerEvent::Activity(WorkerActivity::SubmittedInputDelivered {
            submission_id: id.clone(),
            mode: WorkerSendMode::Queue,
            message: "duplicate text".into(),
        });
        if delivery_first {
            backend.lock().unwrap().events.push_back(delivered);
            responses.extend(project_transport(&mut transport, &mut conversation));
            backend.lock().unwrap().acks.push_back((id.clone(), Ok(())));
        } else {
            backend.lock().unwrap().acks.push_back((id.clone(), Ok(())));
            responses.extend(project_transport(&mut transport, &mut conversation));
            backend.lock().unwrap().events.push_back(delivered);
        }
        responses.extend(project_transport(&mut transport, &mut conversation));
        let prompt_responses = responses
            .iter()
            .filter(|r| r.id.as_deref() == Some(&id))
            .collect::<Vec<_>>();
        assert_eq!(prompt_responses.len(), 1);
        assert!(prompt_responses[0].result.is_ok());
        assert_eq!(
            conversation
                .items
                .iter()
                .filter(|item| item.kind == TranscriptKind::User)
                .count(),
            1
        );
        assert!(conversation.items[0].label.is_empty());
        assert!(
            conversation.queue.follow_up.is_empty(),
            "late acknowledgement cannot revive an aborted queue"
        );
        assert_eq!(
            backend.lock().unwrap().requests.len(),
            1,
            "uncertain receipt never causes replay"
        );
    }
}

#[test]
fn disconnected_submission_is_unknown_but_explicit_rejection_is_not() {
    use crate::app::views::transcript::conversation::ConversationState;
    for rejected in [false, true] {
        let backend = Arc::new(std::sync::Mutex::new(ControlledPromptState::default()));
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "failure".into(),
            Box::new(ControlledPromptWorker(backend.clone())),
            MainSessionMetadata::default(),
            None,
        )
        .unwrap();
        let mut conversation = ConversationState::default();
        let id = transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Steer,
                message: "preserve me".into(),
                images: Vec::new(),
            })
            .unwrap();
        transport.send(SessionCommand::Abort).unwrap();
        project_transport(&mut transport, &mut conversation);
        if rejected {
            backend
                .lock()
                .unwrap()
                .acks
                .push_back((id.clone(), Err("invalid input".into())));
        } else {
            backend
                .lock()
                .unwrap()
                .events
                .push_back(WorkerEvent::Failed("connection lost".into()));
        }
        let responses = project_transport(&mut transport, &mut conversation);
        let response = responses
            .iter()
            .find(|r| r.id.as_deref() == Some(&id))
            .expect("prompt outcome");
        let error = response.result.as_ref().unwrap_err();
        assert_eq!(
            error.kind,
            if rejected {
                crate::agents::SessionResponseErrorKind::RejectedBeforeAcceptance
            } else {
                crate::agents::SessionResponseErrorKind::DeliveryUnknown
            }
        );
        assert_eq!(conversation.items.len(), usize::from(!rejected));
        if !rejected {
            assert_eq!(conversation.items[0].label, "Delivery unknown");
        }
    }
}

#[test]
fn equal_text_submissions_stay_distinct_through_out_of_order_receipts_and_abort() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    let backend = Arc::new(std::sync::Mutex::new(ControlledPromptState::default()));
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "same".into(),
        Box::new(ControlledPromptWorker(backend.clone())),
        MainSessionMetadata::default(),
        None,
    )
    .unwrap();
    let mut conversation = ConversationState::default();
    let first = transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Steer,
            message: "same text".into(),
            images: vec![crate::protocol::PromptImage::new(
                "AQID".into(),
                "image/png".into(),
            )],
        })
        .unwrap();
    let second = transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Steer,
            message: "same text".into(),
            images: vec![],
        })
        .unwrap();
    backend
        .lock()
        .unwrap()
        .acks
        .push_back((second.clone(), Ok(())));
    project_transport(&mut transport, &mut conversation);
    transport.send(SessionCommand::Abort).unwrap();
    project_transport(&mut transport, &mut conversation);
    backend
        .lock()
        .unwrap()
        .events
        .push_back(WorkerEvent::Activity(
            WorkerActivity::SubmittedInputDelivered {
                submission_id: first.clone(),
                mode: WorkerSendMode::Steer,
                message: "same text".into(),
            },
        ));
    project_transport(&mut transport, &mut conversation);
    backend.lock().unwrap().acks.push_back((first, Ok(())));
    backend
        .lock()
        .unwrap()
        .events
        .push_back(WorkerEvent::Activity(
            WorkerActivity::SubmittedInputDelivered {
                submission_id: second,
                mode: WorkerSendMode::Steer,
                message: "same text".into(),
            },
        ));
    project_transport(&mut transport, &mut conversation);
    let users = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User)
        .collect::<Vec<_>>();
    assert_eq!(users.len(), 2);
    assert_eq!(users.iter().map(|item| item.images.len()).sum::<usize>(), 1);
    assert!(
        users
            .iter()
            .all(|item| item.text == "same text" && item.label.is_empty())
    );
    assert!(conversation.queue.steering.is_empty());
    assert_eq!(backend.lock().unwrap().requests.len(), 2);
}

#[test]
fn replacement_transports_do_not_reuse_persisted_submission_ids() {
    let submit = || {
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            "codex-cli",
            "same".into(),
            Box::new(IdleWorker),
            MainSessionMetadata::default(),
            None,
        )
        .unwrap();
        transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Steer,
                message: "next".into(),
                images: vec![],
            })
            .unwrap()
    };
    assert_ne!(submit(), submit());
}

#[test]
fn normal_receipt_and_user_echo_share_identity_and_emit_delivery_evidence() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    let backend = Arc::new(std::sync::Mutex::new(ControlledPromptState::default()));
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "normal".into(),
        Box::new(ControlledPromptWorker(backend.clone())),
        MainSessionMetadata::default(),
        None,
    )
    .unwrap();
    let mut conversation = ConversationState::default();
    let item = conversation.push_local_user_with_prompt_images("normal text".into(), &[], false);
    let id = transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Normal,
            message: "normal text".into(),
            images: vec![],
        })
        .unwrap();
    conversation.bind_submitted_prompt(&id, &item);
    backend.lock().unwrap().acks.push_back((id.clone(), Ok(())));
    project_transport(&mut transport, &mut conversation);
    assert_eq!(conversation.items.len(), 1);
    backend
        .lock()
        .unwrap()
        .events
        .push_back(WorkerEvent::Activity(
            WorkerActivity::SubmittedInputDelivered {
                submission_id: id.clone(),
                mode: WorkerSendMode::Prompt,
                message: "normal text".into(),
            },
        ));
    let mut delivered_ids = Vec::new();
    while let Some(event) = transport.poll() {
        if let SessionEvent::Activity(event) = event {
            let event = event.value();
            if event["type"] == "prompt_delivery" && event["status"] == "delivered" {
                delivered_ids.push(event["submissionId"].as_str().unwrap().to_owned());
            }
            conversation.reduce(event);
        }
    }
    assert_eq!(delivered_ids, [id]);
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User)
            .count(),
        1
    );
    assert!(conversation.queue.steering.is_empty() && conversation.queue.follow_up.is_empty());
    assert!(transport.prompt_deliveries.is_empty());
}

#[test]
fn acknowledged_queue_survives_abort_in_transcript_with_attachments() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    let backend = Arc::new(std::sync::Mutex::new(ControlledPromptState::default()));
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-abort".into(),
        Box::new(ControlledPromptWorker(backend.clone())),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    let mut conversation = ConversationState::default();
    let input = crate::protocol::PromptImage::new("AQID".into(), "image/png".into());
    let id = transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Steer,
            message: "keep this accepted instruction".into(),
            images: vec![input.clone()],
        })
        .expect("submit");
    backend.lock().unwrap().acks.push_back((id.clone(), Ok(())));
    project_transport(&mut transport, &mut conversation);
    transport.send(SessionCommand::Abort).expect("abort");
    while let Some(event) = transport.poll() {
        if let SessionEvent::Activity(event) = event {
            conversation.reduce(event.value());
        }
    }
    let users = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User)
        .collect::<Vec<_>>();
    assert_eq!(
        users.len(),
        1,
        "backend acknowledgement must leave a transcript row even if Abort precedes item delivery"
    );
    assert_eq!(users[0].text, "keep this accepted instruction");
    assert_eq!(users[0].images.len(), 1);
    conversation.replace_history(&[
        json!({"role":"assistant", "content":[{"type":"text", "text":"old response"}]}),
    ]);
    assert_eq!(
        conversation.items.len(),
        2,
        "tracked receipt survives history refresh"
    );
    assert_eq!(conversation.items[1].text, "keep this accepted instruction");
    assert_eq!(conversation.items[1].images.len(), 1);
    transport.enqueue_activity(WorkerActivity::SubmittedInputDeliveredWithImages {
        submission_id: id,
        mode: WorkerSendMode::Steer,
        message: "keep this accepted instruction".into(),
        images: vec![input],
    });
    while let Some(event) = transport.poll() {
        if let SessionEvent::Activity(event) = event {
            conversation.reduce(event.value());
        }
    }
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::User)
            .count(),
        1,
        "late delivery must update the acknowledged row, not duplicate it"
    );
}

#[test]
fn delivered_image_only_prompt_survives_transcript_finalization() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};
    let image = crate::protocol::PromptImage::new("AQID".into(), "image/png".into());
    let mut conversation = ConversationState::default();
    conversation.push_local_user_with_prompt_images(
        String::new(),
        std::slice::from_ref(&image),
        false,
    );
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "claude",
        "one".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("test operation should succeed");
    transport.enqueue_worker_event(WorkerEvent::Activity(
        WorkerActivity::InputDeliveredWithImages {
            mode: WorkerSendMode::Prompt,
            message: String::new(),
            images: vec![image],
        },
    ));
    for event in &transport.pending {
        if let SessionEvent::Activity(event) = event {
            conversation.reduce(event.value());
        }
    }
    let users = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::User)
        .collect::<Vec<_>>();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].images.len(), 1);
}

struct RejectAfterWriteWorker {
    events: VecDeque<WorkerEvent>,
}

impl WorkerSession for RejectAfterWriteWorker {
    fn send(&mut self, _: String, _: WorkerSendMode) -> Result<(), String> {
        self.events.push_back(WorkerEvent::Failed(
            "native backend rejected the prompt".into(),
        ));
        Ok(())
    }

    fn submit_prompt(
        &mut self,
        _id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        self.send_with_images(message, mode, images)?;
        Ok(false)
    }

    fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        self.events.pop_front()
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn prompt_response_does_not_precede_worker_rejection() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(RejectAfterWriteWorker {
            events: VecDeque::new(),
        }),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");

    transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Normal,
            message: "do not delete the durable row yet".into(),
            images: Vec::new(),
        })
        .expect("worker write");

    assert!(
        matches!(transport.poll(), Some(SessionEvent::Response(response)) if response.result.is_err()),
        "a successful bridge response currently arrives before the worker reports rejection"
    );
}

#[test]
fn neutral_metadata_events_refresh_session_state_and_modes() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "example",
        "session".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("test operation should succeed");
    transport.enqueue_activity(WorkerActivity::TitleChanged("Generated title".into()));
    assert_eq!(
        transport.state().session_name.as_deref().expect("title"),
        "Generated title"
    );
    assert!(
        matches!(transport.poll(), Some(SessionEvent::Response(response)) if response.operation() == SessionOperation::LoadState)
    );
    transport.enqueue_activity(WorkerActivity::ModeChanged("plan".into()));
    assert!(
        matches!(transport.poll(), Some(SessionEvent::Response(response)) if matches!(&response.result, Ok(Payload::ListModes { selected: Some(selected), .. }) if selected == "plan"))
    );
    transport.enqueue_activity(WorkerActivity::ConfigurationChanged {
        models: vec![json!({"id":"model-fast","name":"Fast","provider":"example"})],
        efforts: vec!["high".into()],
        modes: Vec::new(),
        selected_model: Some(
            json!({"id":"model-fast","provider":"example","contextWindow":1000000}),
        ),
        selected_effort: Some("high".into()),
    });
    assert_eq!(transport.state().model.expect("model").id, "model-fast");
    assert_eq!(
        transport.state().model.expect("model").context_window,
        1000000
    );
    assert_eq!(
        transport.state().thinking_level.as_deref().expect("effort"),
        "high"
    );
    transport.enqueue_activity(WorkerActivity::ServiceTierChanged {
        selected: Some("priority".into()),
        options: vec!["standard".into(), "priority".into()],
    });
    assert_eq!(
        transport.state().service_tier.as_deref().expect("tier"),
        "priority"
    );
    assert_eq!(transport.state().model.expect("model").id, "model-fast");
    transport.enqueue_activity(WorkerActivity::ServiceTierChanged {
        selected: None,
        options: Vec::new(),
    });
    assert!(transport.state().service_tier.is_none());
    assert!(transport.state().service_tiers.is_empty());
}

#[test]
fn two_choice_questions_preserve_their_options() {
    let options = vec!["TypeScript".into(), "Rust".into()];
    assert_eq!(
        interaction(WorkerInput {
            id: "question".into(),
            prompt: "Choose a language".into(),
            options: options.clone(),
            secret: false,
        }),
        ExtensionUiRequest::Select {
            id: "question".into(),
            title: "Choose a language".into(),
            options,
            timeout: None,
        }
    );
}

#[test]
fn native_child_activity_carries_a_backend_locator_without_discovery() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "parent".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("test operation should succeed");
    transport.enqueue_worker_event(WorkerEvent::Activity(
        WorkerActivity::ChildSessionsChanged {
            id: "child".into(),
            title: Some("Reviewer".into()),
            is_running: true,
            outcome: None,
        },
    ));
    let Some(SessionEvent::Activity(event)) = transport.poll() else {
        panic!("child activity")
    };
    assert_eq!(event.value()["child"]["path"], "/locators/codex-cli/child");
    assert_eq!(event.value()["child"]["outcome"], Value::Null);
    assert_eq!(event.value()["child"]["parent_session"], "parent");
    assert_eq!(event.value()["child"]["is_running"], true);

    transport.enqueue_worker_event(WorkerEvent::Activity(
        WorkerActivity::ChildSessionsChanged {
            id: "child".into(),
            title: Some("Reviewer".into()),
            is_running: false,
            outcome: Some(crate::agents::ChildSessionOutcome::Failed),
        },
    ));
    let Some(SessionEvent::Activity(event)) = transport.poll() else {
        panic!("failed child activity")
    };
    assert_eq!(event.value()["child"]["outcome"], "failed");
}

struct SteeringWorker(WorkerSendMode, Arc<AtomicBool>);

impl WorkerSession for SteeringWorker {
    fn apply_steering(&mut self) -> Result<(), String> {
        self.1.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn send(&mut self, _: String, mode: WorkerSendMode) -> Result<(), String> {
        assert_eq!(mode, self.0);
        Ok(())
    }

    fn submit_prompt(
        &mut self,
        _id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        self.send_with_images(message, mode, images)?;
        Ok(true)
    }

    fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        panic!("applying steering must not abort the worker")
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        None
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn applying_steering_preserves_the_running_worker_and_pending_delivery() {
    for (harness, mode) in [
        ("codex-cli", WorkerSendMode::Steer),
        ("opencode2", WorkerSendMode::Steer),
        ("cursor-cli", WorkerSendMode::Queue),
        ("claude", WorkerSendMode::Queue),
    ] {
        let applied = Arc::new(AtomicBool::new(false));
        let mut transport = WorkerSessionTransport::new(
            std::path::Path::new("/locators"),
            harness,
            "session-1".into(),
            Box::new(SteeringWorker(mode, applied.clone())),
            MainSessionMetadata::default(),
            None,
        )
        .expect("transport");
        transport.enqueue_worker_event(WorkerEvent::Started);
        transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Steer,
                message: "redirect".into(),
                images: vec![],
            })
            .expect("send steer");
        transport
            .send(SessionCommand::ApplySteering)
            .expect("apply steer");
        assert!(applied.load(Ordering::SeqCst));
        assert!(transport.running);
        let (queued, other) = if mode == WorkerSendMode::Queue {
            (&transport.follow_up, &transport.steering)
        } else {
            (&transport.steering, &transport.follow_up)
        };
        assert_eq!(queued, &["redirect"]);
        assert!(other.is_empty());
        transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::InputDelivered {
            mode,
            message: "redirect".into(),
        }));
        assert!(transport.steering.is_empty());
        assert!(transport.follow_up.is_empty());
    }
}

struct DeliveryBeforeAckWorker {
    polls: usize,
    acknowledged: bool,
    submitted_id: Option<String>,
}

impl WorkerSession for DeliveryBeforeAckWorker {
    fn send(&mut self, _: String, _: WorkerSendMode) -> Result<(), String> {
        Ok(())
    }

    fn submit_prompt(
        &mut self,
        id: String,
        _: String,
        _: WorkerSendMode,
        _: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        self.submitted_id = Some(id);
        Ok(false)
    }

    fn poll_prompt_ack(&mut self) -> Option<(String, Result<(), String>)> {
        self.acknowledged
            .then(|| {
                (
                    self.submitted_id.clone().expect("submitted request"),
                    Ok(()),
                )
            })
            .inspect(|_| self.acknowledged = false)
    }

    fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        self.polls += 1;
        match self.polls {
            1 => Some(WorkerEvent::Activity(WorkerActivity::InputDelivered {
                mode: WorkerSendMode::Queue,
                message: "next task".into(),
            })),
            2 => {
                self.acknowledged = true;
                None
            }
            _ => None,
        }
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn delivery_before_ack_does_not_restore_a_completed_follow_up() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "claude",
        "session-1".into(),
        Box::new(DeliveryBeforeAckWorker {
            polls: 0,
            acknowledged: false,
            submitted_id: None,
        }),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::FollowUp,
            message: "next task".into(),
            images: Vec::new(),
        })
        .expect("submit follow-up");

    while transport.poll().is_some() {}

    assert!(transport.follow_up.is_empty());
}

#[test]
fn queue_tracking_correlates_real_ids_across_event_orders_and_rejection() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "claude",
        "session-1".into(),
        Box::new(DeliveryBeforeAckWorker {
            polls: usize::MAX,
            acknowledged: false,
            submitted_id: None,
        }),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    let submit = |transport: &mut WorkerSessionTransport, message: &str| {
        transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::FollowUp,
                message: message.into(),
                images: Vec::new(),
            })
            .expect("submit follow-up")
    };

    let ack_first = submit(&mut transport, "same text");
    transport.finish_prompt_ack(ack_first, Ok(()));
    transport.enqueue_activity(WorkerActivity::InputDelivered {
        mode: WorkerSendMode::Queue,
        message: "same text".into(),
    });
    assert!(transport.follow_up.is_empty());

    let first = submit(&mut transport, "same text");
    let second = submit(&mut transport, "same text");
    transport.finish_prompt_ack(second.clone(), Ok(()));
    transport.enqueue_activity(WorkerActivity::SubmittedInputDelivered {
        submission_id: "unknown-submission".into(),
        mode: WorkerSendMode::Queue,
        message: "same text".into(),
    });
    assert_eq!(transport.follow_up, ["same text"]);
    transport.enqueue_activity(WorkerActivity::SubmittedInputDelivered {
        submission_id: second,
        mode: WorkerSendMode::Queue,
        message: "same text".into(),
    });
    assert!(transport.follow_up.is_empty());
    assert_eq!(transport.prompt_deliveries[0].request_id, first);
    transport.finish_prompt_ack(first, Err("first request rejected".into()));
    assert!(transport.follow_up.is_empty());

    let rejected = submit(&mut transport, "rejected text");
    transport.finish_prompt_ack(rejected, Err("backend rejected it".into()));
    assert!(
        !transport
            .follow_up
            .iter()
            .any(|item| item == "rejected text")
    );
}

#[test]
fn request_local_failure_is_visible_without_failing_the_transport() {
    use crate::app::views::transcript::conversation::ConversationState;

    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    transport.enqueue_worker_event(WorkerEvent::RequestFailed {
        operation: "Codex interrupt".into(),
        error: "turn is no longer active".into(),
    });
    let mut conversation = ConversationState::default();
    while let Some(event) = transport.pending.pop_front() {
        assert!(!matches!(event, SessionEvent::Failure(_)));
        if let SessionEvent::Activity(activity) = event {
            conversation.reduce(activity.value());
        }
    }
    assert!(conversation.items.iter().any(|item| {
        item.complete_text()
            .contains("Codex interrupt: turn is no longer active")
    }));
}

#[test]
fn settlement_preserves_an_undelivered_follow_up() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::FollowUp,
            message: "next task".into(),
            images: Vec::new(),
        })
        .expect("queue follow-up");
    transport.enqueue_worker_event(WorkerEvent::Settled {
        output: "first turn done".into(),
    });

    assert_eq!(transport.follow_up, ["next task"]);
}

#[test]
fn repeated_started_during_a_stream_does_not_duplicate_visible_text() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};

    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "opencode2",
        "session-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    transport.enqueue_worker_event(WorkerEvent::Started);
    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
        content_index: 0,
        delta: "hello ".into(),
    }));
    transport.enqueue_worker_event(WorkerEvent::Started);
    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
        content_index: 0,
        delta: "world".into(),
    }));
    transport.enqueue_worker_event(WorkerEvent::Settled {
        output: "hello world".into(),
    });

    let mut conversation = ConversationState::default();
    for event in transport.pending {
        if let SessionEvent::Activity(event) = event {
            conversation.reduce(event.value());
        }
    }
    let assistant = conversation
        .items
        .iter()
        .filter(|item| item.kind == TranscriptKind::Assistant)
        .map(|item| item.complete_text())
        .collect::<Vec<_>>();
    assert_eq!(assistant, ["hello world"]);
}

#[test]
fn started_after_settlement_begins_a_real_new_assistant_turn() {
    use crate::app::views::transcript::conversation::{ConversationState, TranscriptKind};

    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    for output in ["first", "second"] {
        transport.enqueue_worker_event(WorkerEvent::Started);
        transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
            content_index: 0,
            delta: output.into(),
        }));
        transport.enqueue_worker_event(WorkerEvent::Settled {
            output: output.into(),
        });
    }
    let mut conversation = ConversationState::default();
    for event in transport.pending {
        if let SessionEvent::Activity(event) = event {
            conversation.reduce(event.value());
        }
    }
    assert_eq!(
        conversation
            .items
            .iter()
            .filter(|item| item.kind == TranscriptKind::Assistant)
            .map(|item| item.complete_text())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

impl WorkerSession for IdleWorker {
    fn send_with_images(
        &mut self,
        message: String,
        mode: WorkerSendMode,
        _: Vec<crate::protocol::PromptImage>,
    ) -> Result<(), String> {
        self.send(message, mode)
    }

    fn send(&mut self, _: String, _: WorkerSendMode) -> Result<(), String> {
        Ok(())
    }

    fn submit_prompt(
        &mut self,
        _id: String,
        message: String,
        mode: WorkerSendMode,
        images: Vec<crate::protocol::PromptImage>,
    ) -> Result<bool, String> {
        self.send_with_images(message, mode, images)?;
        Ok(true)
    }

    fn respond(&mut self, _: WorkerInputResponse) -> Result<(), String> {
        Ok(())
    }

    fn abort(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn rename(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        None
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn resume_locator_comes_from_the_external_session_path_when_the_runtime_has_no_id() {
    let path = external_session_path(
        std::path::Path::new("/locators"),
        "opencode2",
        "session/one",
    );
    let launch = crate::agents::SessionLaunch {
        harness: "opencode2".into(),
        session_id: None,
        project: "/project".into(),
        start: crate::agents::SessionStart::Resume(path),
        wake: None,
    };

    assert_eq!(
        launch_session_locator(&launch).as_deref(),
        Some("session/one")
    );
}

#[test]
fn text_around_tools_is_emitted_as_chronological_messages() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");

    transport.enqueue_worker_event(WorkerEvent::Started);
    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
        content_index: 0,
        delta: "before".into(),
    }));
    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::ToolStarted {
        id: "tool-1".into(),
        name: "command".into(),
        args: json!({}),
        metadata: Default::default(),
    }));
    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::TextDelta {
        content_index: 0,
        delta: "after".into(),
    }));
    transport.enqueue_worker_event(WorkerEvent::Settled {
        output: "beforeafter".into(),
    });

    let activities = transport
        .pending
        .into_iter()
        .filter_map(|event| match event {
            SessionEvent::Activity(activity) => Some(activity),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        activities
            .iter()
            .map(|activity| activity.value()["type"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        [
            "agent_start",
            "message_start",
            "message_update",
            "message_end",
            "tool_execution_start",
            "message_start",
            "message_update",
            "message_end",
            "agent_settled",
        ]
    );
    assert_eq!(
        activities[3].value()["message"]["content"][0]["text"],
        "before"
    );
    assert_eq!(
        activities[7].value()["message"]["content"][0]["text"],
        "after"
    );
}

#[test]
fn delivered_worker_message_leaves_the_queue_and_enters_the_transcript() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");

    transport.steering.push("redirect".into());
    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::InputDelivered {
        mode: WorkerSendMode::Steer,
        message: "redirect".into(),
    }));
    assert!(transport.steering.is_empty());

    let delivered = transport
        .pending
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Activity(activity)
                if matches!(
                    activity.value()["type"].as_str(),
                    Some("message_start" | "message_end")
                ) =>
            {
                Some(activity.value().clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        delivered,
        [
            json!({"type": "message_start", "message": {"role": "user", "content": "redirect", "queued": true}}),
            json!({"type": "message_end", "message": {"role": "user", "content": "redirect", "queued": true}}),
        ]
    );
}

#[test]
fn peer_delivery_is_a_first_class_activity_instead_of_a_user_message() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");

    transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::PeerInputDelivered {
        message: crate::agents::PeerMessage {
            from: "worker-7".into(),
            message: "review complete".into(),
        },
    }));

    let event = transport.pending.pop_front().expect("peer event");
    let SessionEvent::Activity(activity) = event else {
        panic!("expected activity");
    };
    assert_eq!(activity.value()["type"], "peer_message");
    assert_eq!(activity.value()["from"], "worker-7");
    assert_eq!(activity.value()["message"], "review complete");
}

#[test]
fn worker_session_state_retains_titles_and_counts_new_messages() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "session-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        None,
    )
    .expect("transport");
    assert_eq!(transport.state().message_count, 0);
    assert!(transport.state().session_name.is_none());

    for turn in 0..2 {
        let prompt = format!("Request {turn}");
        transport
            .send(SessionCommand::Prompt {
                mode: PromptMode::Normal,
                message: prompt.clone(),
                images: Vec::new(),
            })
            .expect("send prompt");
        assert_eq!(transport.state().message_count, turn * 2 + 1);
        transport.enqueue_worker_event(WorkerEvent::Started);
        transport.enqueue_worker_event(WorkerEvent::Activity(WorkerActivity::InputDelivered {
            mode: WorkerSendMode::Prompt,
            message: prompt,
        }));
        transport.enqueue_worker_event(WorkerEvent::Settled {
            output: "Done".into(),
        });
        if turn == 0 {
            transport
                .send(SessionCommand::Rename {
                    name: "Generated title".into(),
                })
                .expect("rename");
        }
        transport.pending.clear();
        transport
            .send(SessionCommand::LoadState)
            .expect("load state");
        let Some(SessionEvent::Response(response)) = transport.poll() else {
            panic!("expected state response");
        };
        let state = state_of(response.result.expect("state response"));
        assert_eq!(state.session_name.as_deref(), Some("Generated title"));
        assert_eq!(state.message_count, (turn + 1) * 2);
    }
}

#[test]
fn resumed_transport_returns_persisted_history() {
    let history = crate::agents::DiscoveredHistory {
        messages: vec![json!({"role": "user", "content": "persisted"})],
        model: Some(("openai".into(), "gpt-test".into())),
        thinking_level: Some("high".into()),
    };
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "codex-cli",
        "thread-1".into(),
        Box::new(IdleWorker),
        MainSessionMetadata::default(),
        Some(history),
    )
    .expect("transport");

    transport
        .send(SessionCommand::LoadHistory)
        .expect("load history");
    let SessionEvent::Response(response) = transport.poll().expect("history response") else {
        panic!("expected history response");
    };
    assert_eq!(response.operation(), SessionOperation::LoadHistory);
    let Ok(Payload::LoadHistory(SessionHistory::Replace(messages))) = response.result else {
        panic!("expected replacement history");
    };
    assert_eq!(messages[0]["content"], "persisted");

    transport
        .send(SessionCommand::LoadState)
        .expect("load state");
    let SessionEvent::Response(response) = transport.poll().expect("state response") else {
        panic!("expected state response");
    };
    let state = state_of(response.result.expect("state response"));
    assert_eq!(state.message_count, 1);
    assert_eq!(state.model.expect("model").id, "gpt-test");
    assert_eq!(state.thinking_level.as_deref(), Some("high"));

    transport
        .send(SessionCommand::Prompt {
            mode: PromptMode::Normal,
            message: "New request".into(),
            images: Vec::new(),
        })
        .expect("send prompt");
    assert_eq!(transport.state().message_count, 2);
}

#[test]
fn a_new_transport_without_a_picked_effort_reports_no_level() {
    let metadata = MainSessionMetadata {
        models: vec![json!({
            "id": "gpt-test",
            "provider": "openai",
            "reasoning": true,
        })],
        ..MainSessionMetadata::default()
    };
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "opencode2",
        "thread-1".into(),
        Box::new(IdleWorker),
        metadata,
        None,
    )
    .expect("transport");

    transport
        .send(SessionCommand::LoadState)
        .expect("load state");
    let SessionEvent::Response(response) = transport.poll().expect("state response") else {
        panic!("expected state response");
    };
    assert!(
        state_of(response.result.expect("state response"))
            .thinking_level
            .is_none()
    );
}

#[test]
fn completion_is_an_authoritative_message_before_settling() {
    let mut message = AssistantMessage::default();
    message.append_delta(0, "thinking", "thinking", "plan");
    message.append_delta(1, "text", "text", "partial");
    message.replace_text("final");
    assert_eq!(
        message.content(),
        vec![
            json!({"type": "thinking", "thinking": "plan"}),
            json!({"type": "text", "text": "final"}),
        ]
    );
}

fn state_of(payload: Payload) -> SessionState {
    let Payload::LoadState(state) = payload else {
        panic!("expected state")
    };
    *state
}

#[test]
fn malformed_worker_catalogs_fail_without_dropping_invalid_entries() {
    let mut transport = WorkerSessionTransport::new(
        std::path::Path::new("/locators"),
        "example",
        "session".into(),
        Box::new(IdleWorker),
        MainSessionMetadata {
            models: vec![
                json!({"id":"valid","name":"Valid","provider":"example"}),
                json!({"id":"bad"}),
            ],
            modes: vec![json!({"id":"plan"})],
            commands: vec![json!({"name":"review","source":"unknown"})],
            ..Default::default()
        },
        None,
    )
    .expect("transport");
    for command in [
        SessionCommand::ListModels,
        SessionCommand::ListModes,
        SessionCommand::ListCommands,
    ] {
        let operation = command.response_operation();
        let id = transport.send(command).expect("request");
        let Some(SessionEvent::Response(response)) = transport.poll() else {
            panic!("response")
        };
        assert_eq!(response.id.as_deref(), Some(id.as_str()));
        assert_eq!(response.operation(), operation);
        assert!(response.result.is_err());
    }
    transport
        .send(SessionCommand::LoadState)
        .expect("state request");
    assert!(
        matches!(transport.poll(), Some(SessionEvent::Response(response)) if response.result.is_ok())
    );
}
