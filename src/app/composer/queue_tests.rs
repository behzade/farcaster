use super::*;

#[test]
fn queued_message_preview_hides_multiline_payloads() {
    for (message, expected) in [
        (
            "inspect this\n\nPasted text files:\n- file.txt",
            "inspect this…",
        ),
        (" \r\n\t ", "Message"),
        ("  سلام  \r\nmore\n", "سلام…"),
        (" single line \n\t", "single line"),
        (
            "Message from Farcaster worker review:\n\n done \r\nmore",
            "review:  done…",
        ),
        (
            "Message from Farcaster peer worker-7:\n\nlegacy\n",
            "worker-7: legacy",
        ),
        ("Message from Farcaster worker review:\n\n \t", "review:"),
        (
            "Message from Farcaster worker review:\n\n\nnext",
            "review:…",
        ),
        (
            "Message from Farcaster worker bad id:\n\nbody",
            "Message from Farcaster worker bad id:…",
        ),
    ] {
        assert_eq!(display_text(message).0, expected, "{message:?}");
    }
}

#[test]
fn peer_messages_keep_their_delivery_group_and_sender_preview() {
    let peer = "Message from Farcaster peer worker-7:\n\nreview complete\nwith details".to_owned();
    let queue = QueueState {
        steering: vec![peer.clone(), "redirect now".into()],
        follow_up: vec![peer.clone()],
        ..Default::default()
    };

    let view = prepare(&queue, &[], "target", None, true, false);
    assert_eq!(view.groups.len(), 2);
    assert_eq!(view.groups[0].heading.unwrap().0, "Steer");
    assert_eq!(view.groups[0].rows.len(), 2);
    assert_eq!(view.groups[1].heading.unwrap().0, "Follow-up");
    for group in &view.groups {
        assert_eq!(group.rows[0].preview, "worker-7: review complete…");
        assert_eq!(
            group.rows[0].body,
            "worker-7: review complete\nwith details"
        );
    }
}

#[test]
fn prepared_actions_preserve_exact_ids_and_receipt_ownership() {
    let queue = QueueState {
        steering: vec!["same".into(), "same".into()],
        steering_ids: vec!["owned".into(), "inflight".into()],
        cancellable_ids: vec!["owned".into()],
        ..Default::default()
    };
    let receipts = [
        crate::conversation::PendingReceipt {
            id: "owned".into(),
            text: "same".into(),
            mode: Some(PromptMode::Steer),
            images: Default::default(),
            unknown: false,
        },
        crate::conversation::PendingReceipt {
            id: "inflight".into(),
            text: "same".into(),
            mode: Some(PromptMode::Steer),
            images: Default::default(),
            unknown: true,
        },
        crate::conversation::PendingReceipt {
            id: "restored".into(),
            text: "again".into(),
            mode: Some(PromptMode::FollowUp),
            images: Default::default(),
            unknown: true,
        },
    ];
    let receipts = receipts
        .each_ref()
        .map(crate::conversation::PendingReceipt::as_ref);
    let session = Path::new("/session");
    let view = prepare(&queue, &receipts, "target", Some(session), true, true);
    let rows = &view.groups[0].rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(view.groups[1].rows.len(), 1);
    assert_ne!(rows[0].id, rows[1].id);
    assert!(rows[0].notice.is_none());
    assert_eq!(rows[1].notice, Some("Delivery not confirmed"));
    assert!(rows[1].action.is_none());
    assert!(matches!(rows[0].action.as_ref().unwrap().runtime_command(),
        Some(RuntimeCommand::CancelQueued { target, id }) if target == "target" && id == "owned"));
    assert!(
        matches!(view.groups[1].rows[0].action.as_ref().unwrap().runtime_command(),
        Some(RuntimeCommand::DismissReceipt { session: path, id }) if path == session && id == "restored")
    );
    let no_session = prepare(&queue, &receipts, "target", None, true, true);
    assert!(no_session.groups[1].rows[0].action.is_none());
    let live = prepare(&queue, &receipts, "target", Some(session), true, false);
    assert_eq!(live.groups.len(), 1);
    let bulk = prepare(&queue, &receipts, "target", Some(session), false, false);
    assert!(bulk.groups[0].rows.iter().all(|row| row.action.is_none()));
    assert!(matches!(
        bulk.clear.unwrap().runtime_command(),
        Some(RuntimeCommand::ClearQueue)
    ));
}

#[test]
fn recovery_keeps_saved_target_and_disables_unsendable_prompts() {
    let mut queue = QueueState {
        saved: vec![crate::conversation::SavedPrompt {
            id: 42,
            target: "saved-target".into(),
            text: "hello".into(),
            image_count: 2,
            sendable: false,
        }],
        ..Default::default()
    };
    let view = prepare(&queue, &[], "current-target", None, true, false);
    let row = &view.groups[0].rows[0];
    assert_eq!(row.preview, row.body);
    assert!(row.notice.is_some());
    let details = row.details.as_ref().unwrap();
    assert!(details.notice.contains("duplicate"));
    assert_eq!(details.caption.as_deref(), Some("2 image(s) attached"));
    assert!(!details.actions[0].enabled);
    assert!(details.actions[0].runtime_command().is_none());
    assert!(matches!(details.actions[1].runtime_command(),
        Some(RuntimeCommand::RemoveSaved { target, id: 42 }) if target == "saved-target"));
    queue.saved[0].sendable = true;
    let view = prepare(&queue, &[], "current-target", None, true, false);
    assert!(
        matches!(view.groups[0].rows[0].details.as_ref().unwrap().actions[0].runtime_command(),
        Some(RuntimeCommand::SendSaved { target, id: 42 }) if target == "saved-target")
    );
}

#[test]
fn display_text_compares_visible_content_without_transport_or_outer_whitespace() {
    for text in [
        "hi",
        " single line \n\t",
        " \r\n\t ",
        "سلام",
        "Message from Farcaster worker review:\n\n done \n",
    ] {
        let (preview, body) = display_text(text);
        assert_eq!(preview, body, "{text:?}");
    }
    let (preview, body) = display_text("Message from Farcaster worker review:\n\nfirst\nsecond");
    assert_eq!(preview, "review: first…");
    assert_eq!(body, "review: first\nsecond");
}

#[gpui::test]
fn queue_actions_follow_the_selected_session_snapshot(cx: &mut gpui::TestAppContext) {
    use crate::{
        agents::Backend,
        conversation::SavedPrompt,
        runtime::{HarnessAccessMode, RuntimeEvent, RuntimeSignal, RuntimeSnapshot},
        sessions::{SessionSummary, session_buffer, session_target},
    };
    use std::sync::Arc;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::queue_actions_follow_the_selected_session_snapshot"
        ),
        cx,
        |cx, app, runtime, project| {
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            let path = project.join("shared-locator");
            let first = SessionSummary::from_cached_for_harness(
                "first".into(),
                Backend::Pi,
                path.clone(),
                project.into(),
                "First".into(),
                String::new(),
                String::new(),
                None,
                std::time::UNIX_EPOCH,
                0,
                Default::default(),
                false,
                false,
                String::new(),
            )
            .with_app_session_id(1);
            let mut second = first.clone().with_app_session_id(2);
            second.harness = Backend::Codex;
            second.id = "second".into();

            let snapshot = |row: &SessionSummary| {
                Arc::new(RuntimeSnapshot {
                    harness: Some(row.harness),
                    selected_session: Some(path.clone()),
                    project: project.into(),
                    prompt_queue: Some(Arc::new(QueueState {
                        saved: vec![SavedPrompt {
                            id: row.app_session_id,
                            target: session_target(&path),
                            text: row.id.clone(),
                            image_count: 0,
                            sendable: true,
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                })
            };
            let remove = |app_state: &FarcasterApp| {
                let (view, handler) = app_state
                    .composer_queue(app.downgrade())
                    .expect("visible queue");
                let row = &view.groups[0].rows[0];
                let action = row
                    .details
                    .as_ref()
                    .unwrap()
                    .actions
                    .iter()
                    .find(|action| action.label == "Remove")
                    .unwrap()
                    .clone();
                (handler, action)
            };
            let (old_handler, old_action) = cx.update(|window, cx| {
                app.update(cx, |state, cx| {
                    let rows = vec![first.clone(), second.clone()];
                    state.sessions.all = rows.clone().into();
                    state.sessions.visible = rows.into();
                    state.sessions.selected_draft = None;
                    state.switch_composer_target(session_buffer(first.harness, &path), window, cx);
                    state.snapshot = snapshot(&first);
                    let old = remove(state);
                    state.select_rail_session(second.catalog_key(), window, cx);
                    assert!(state.composer_queue(app.downgrade()).is_none());
                    state.set_access_mode(HarnessAccessMode::Full, cx);
                    state.signal_composer(RuntimeSignal::Abort);
                    old
                })
            });
            cx.update(|_, cx| old_handler(&old_action, cx));
            cx.run_until_parked();
            let mut request = None;
            for command in std::iter::from_fn(|| runtime.try_recv_command()) {
                match command {
                    RuntimeCommand::SelectSession {
                        request: sequence,
                        key,
                        ..
                    } if key == Some(second.catalog_key()) => request = sequence,
                    RuntimeCommand::RemoveSaved { .. } | RuntimeCommand::SetAccessMode(_) => {
                        panic!("stale action dispatched before routing")
                    }
                    _ => {}
                }
            }
            assert!(runtime.try_recv_signal().is_none());
            runtime.send_event(RuntimeEvent::SelectionRouted(
                request.expect("selection dispatched"),
            ));
            cx.update(|_, cx| {
                app.update(cx, |state, cx| {
                    state.drain_runtime(cx);
                    assert!(state.composer_queue(app.downgrade()).is_none());
                    state.signal_composer(RuntimeSignal::Abort);
                    assert!(runtime.try_recv_signal().is_none());
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: state.runtime_generation + 1,
                        snapshot: snapshot(&second),
                    });
                    state.drain_runtime(cx);
                    state.set_access_mode(HarnessAccessMode::Full, cx);
                    state.signal_composer(RuntimeSignal::Abort);
                    assert_eq!(
                        runtime.try_recv_signal(),
                        Some((session_buffer(second.harness, &path), RuntimeSignal::Abort))
                    );
                    assert!(std::iter::from_fn(|| runtime.try_recv_command()).any(
                        |command| matches!(
                            command,
                            RuntimeCommand::SetAccessMode(HarnessAccessMode::Full)
                        )
                    ));
                });
                let (new_handler, new_action) = remove(app.read(cx));
                old_handler(&old_action, cx);
                new_handler(&new_action, cx);
            });
            let removed: Vec<_> = std::iter::from_fn(|| runtime.try_recv_command())
                .filter_map(|command| match command {
                    RuntimeCommand::RemoveSaved { target, id } => Some((target, id)),
                    _ => None,
                })
                .collect();
            assert_eq!(removed, [(session_target(&path), second.app_session_id)]);
        },
    );
}
