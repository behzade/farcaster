use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::sessions::{SessionSummary, UsageSummary};

fn session(path: &str, archived: bool) -> SessionSummary {
    SessionSummary::from_cached(
        "test".into(),
        path.into(),
        "/project".into(),
        "Test".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::UNIX_EPOCH,
        0,
        UsageSummary::default(),
        archived,
        false,
        String::new(),
    )
}

fn pending() -> PendingSubmission {
    PendingSubmission {
        id: "pending-submission".into(),
        submitted_at: Instant::now(),
        submitted_target: "session:compacting".into(),
        mode: PromptMode::Steer,
        text: "submitted".into(),
        images: Vec::new(),
        pastes: Vec::new(),
        append_on_failure: false,
        result: None,
    }
}

#[gpui::test]
fn submission_queue_matches_backend_steering_support(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::submission_queue_matches_backend_steering_support"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let path = project.join("shared.jsonl");
                    app.sessions.selected_draft = None;
                    for (backend, expected_mode) in [
                        (crate::agents::Backend::Antigravity, PromptMode::Steer),
                        (crate::agents::Backend::Cursor, PromptMode::Steer),
                    ] {
                        app.composer.pending_submissions.clear();
                        let buffer = crate::sessions::session_buffer(backend, &path);
                        app.switch_composer_target(buffer.clone(), window, cx);
                        let snapshot = Arc::make_mut(&mut app.snapshot);
                        snapshot.selected_session = Some(path.clone());
                        snapshot.harness = Some(backend);
                        Arc::make_mut(&mut snapshot.conversation).running = true;
                        let mut native = crate::conversation::QueueState::default();
                        match expected_mode {
                            PromptMode::Steer => {
                                native.steering = vec!["same text".into()];
                                native.steering_ids = vec!["backend-input".into()];
                            }
                            _ => {
                                native.follow_up = vec!["same text".into()];
                                native.follow_up_ids = vec!["backend-input".into()];
                            }
                        }
                        for count in 1..=2 {
                            app.composer
                                .input
                                .update(cx, |input, cx| input.set_value("same text", window, cx));
                            app.submit("same text".into(), PromptMode::Steer, window, cx);
                            let pending = &app.composer.pending_submissions;
                            assert_eq!(pending.len(), count);
                            assert!(
                                pending.values().all(|input| input.mode == expected_mode
                                    && input.submitted_target == buffer),
                                "wrong queue for {backend} on submission {count}: {pending:?}"
                            );
                            let (target, mode) = std::iter::from_fn(|| runtime.try_recv_command())
                                .find_map(|command| match command {
                                    RuntimeCommand::Prompt { target, mode, .. } => {
                                        Some((target, mode))
                                    }
                                    _ => None,
                                })
                                .expect("prompt command");
                            assert_eq!(target, session_target(&path));
                            assert_eq!(mode, expected_mode);
                            let visible = visible_prompt_queue(
                                &native,
                                pending,
                                app.composer.sessions.current_buffer(),
                            );
                            let (messages, other) = if expected_mode == PromptMode::Steer {
                                (&visible.steering, &visible.follow_up)
                            } else {
                                (&visible.follow_up, &visible.steering)
                            };
                            assert!(
                                other.is_empty(),
                                "one request appeared in both queues: {visible:?}"
                            );
                            assert_eq!(messages, &vec!["same text"; count]);
                        }
                    }
                });
            });
        },
    );
}

fn image(data: &str) -> ComposerImage {
    ComposerImage {
        prompt: PromptImage::new(data.into(), "image/png".into()),
        preview: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, Vec::new())),
        byte_len: 0,
    }
}

#[gpui::test]
fn first_submit_restores_input_when_enqueue_fails(cx: &mut gpui::TestAppContext) {
    check_rejected_first_prompt(
        concat!(
            module_path!(),
            "::first_submit_restores_input_when_enqueue_fails"
        ),
        cx,
        false,
        None,
    );
}

#[gpui::test]
fn allocated_draft_remains_editable_after_enqueue_fails(cx: &mut gpui::TestAppContext) {
    check_rejected_first_prompt(
        concat!(
            module_path!(),
            "::allocated_draft_remains_editable_after_enqueue_fails"
        ),
        cx,
        true,
        None,
    );
}

#[gpui::test]
fn rejection_and_retry_preserve_a_named_draft(cx: &mut gpui::TestAppContext) {
    check_rejected_first_prompt(
        concat!(
            module_path!(),
            "::rejection_and_retry_preserve_a_named_draft"
        ),
        cx,
        true,
        Some("My title"),
    );
}

fn check_rejected_first_prompt(
    test_name: &str,
    cx: &mut gpui::TestAppContext,
    allocate_before_rejection: bool,
    original_title: Option<&str>,
) {
    use crate::{
        agents::Backend,
        app::persistence,
        runtime::RuntimeEvent,
        storage::{PromptSubmission, SessionRepository as _},
    };

    crate::app::test_support::with_offline_app(test_name, cx, |cx, app, runtime, project| {
        let repository = persistence::shared().unwrap();
        let sql = |sql: &str| {
            repository
                .with(|store| {
                    store.with_connection(|conn| {
                        conn.execute_batch(sql).map_err(|error| error.to_string())
                    })
                })
                .unwrap()
        };
        sql("CREATE TRIGGER reject_prompt BEFORE INSERT ON outbox
             BEGIN SELECT RAISE(FAIL, 'fixture enqueue failure'); END");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let draft = app.sessions.selected_draft.clone().unwrap();
                let target = crate::sessions::draft_target(&draft);
                app.sessions
                    .drafts
                    .iter_mut()
                    .find(|d| d.id == draft)
                    .unwrap()
                    .title = original_title.map(str::to_owned);
                Arc::make_mut(&mut app.snapshot).harness = Some(Backend::Codex);
                let send = |app: &mut FarcasterApp,
                            text: &str,
                            window: &mut Window,
                            cx: &mut Context<FarcasterApp>| {
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
                    app.submit(text.to_owned(), PromptMode::Normal, window, cx);
                    let (submission_id, title) = std::iter::from_fn(|| runtime.try_recv_command())
                        .find_map(|command| match command {
                            RuntimeCommand::Prompt {
                                target: to,
                                submission_id,
                                message,
                                provisional_title,
                                ..
                            } if to == target && message == text => {
                                Some((submission_id, provisional_title))
                            }
                            _ => None,
                        })
                        .expect("prompt sent to runtime");
                    let result = repository.submit_prompt(PromptSubmission {
                        target: &target,
                        harness: Backend::Codex,
                        project,
                        session: None,
                        profile_id: None,
                        mode: PromptMode::Normal,
                        message: text,
                        provisional_title: title.as_deref(),
                        display_message: None,
                        invocation: None,
                        images: &[],
                    });
                    (submission_id, title, result)
                };
                let reject = |submission_id| {
                    runtime.send_event(RuntimeEvent::PromptResult {
                        submission_id: Some(submission_id),
                        target: target.clone(),
                        outcome: crate::agents::PromptOutcome::RejectedBeforeAcceptance,
                        session: None,
                    })
                };
                let (first, _, failed) = send(app, "Prompt A", window, cx);
                assert!(failed.err().unwrap().contains("fixture enqueue failure"));
                assert!(app.composer.input.read(cx).value().is_empty());
                if allocate_before_rejection {
                    let snapshot = app
                        .sessions
                        .drafts
                        .iter()
                        .find(|d| d.id == draft)
                        .unwrap()
                        .clone();
                    let id = repository
                        .with(|db| db.save_draft_with_identity(snapshot))
                        .unwrap();
                    runtime.send_event(RuntimeEvent::DraftIdentitySaved {
                        draft: draft.clone(),
                        app_session_id: id,
                    });
                }
                reject(first);
                app.drain_runtime(cx);
                app.resolve_pending_submission(window, cx);
                assert_eq!(app.composer.input.read(cx).value().as_ref(), "Prompt A");
                assert!(app.composer.pending_submissions.is_empty());
                assert!(app.editable_draft_project().is_some());
                assert!(app.editable_draft_harness().is_some());
                assert!(app.save_draft_before_handoff(&draft));
                futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                let saved = || {
                    repository
                        .with(|db| {
                            Ok(db
                                .load_drafts()?
                                .into_iter()
                                .find(|d| d.id == draft)
                                .unwrap())
                        })
                        .unwrap()
                };
                let before = saved();
                assert!(!before.submitted);
                assert_eq!(before.title.as_deref(), original_title);

                sql("DROP TRIGGER reject_prompt");
                let (second, title, result) = send(app, "Prompt B", window, cx);
                let queued = result.unwrap();
                assert_eq!(queued.app_session_id.get(), before.app_session_id);
                runtime.send_event(RuntimeEvent::DraftPromptCommitted {
                    draft: draft.clone(),
                    app_session_id: queued.app_session_id,
                    provisional_title: title,
                });
                reject(second);
                app.drain_runtime(cx);
                app.resolve_pending_submission(window, cx);
                let row = app.sessions.drafts.iter().find(|d| d.id == draft).unwrap();
                assert!(row.submitted);
                assert_eq!(row.title.as_deref(), original_title.or(Some("Prompt B")));
                assert_eq!(saved().title, row.title);
                assert!(app.editable_draft_harness().is_none());
            })
        });
    });
}

#[gpui::test]
fn earlier_rejection_preserves_later_bound_submission(cx: &mut gpui::TestAppContext) {
    use crate::{
        agents::{Backend, PromptOutcome},
        app::persistence,
        runtime::RuntimeEvent,
        storage::{ObservedSession, PromptSubmission, SessionFacts, SessionRepository as _},
    };
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::earlier_rejection_preserves_later_bound_submission"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = app.sessions.selected_draft.clone().unwrap();
                    let target = crate::sessions::draft_target(&id);
                    let path = crate::sessions::normalize_session_path(&project.join("bound"));
                    Arc::make_mut(&mut app.snapshot).harness = Some(Backend::Codex);
                    for text in ["Prompt A", "Prompt B"] {
                        app.composer
                            .input
                            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
                        app.submit(text.into(), PromptMode::Normal, window, cx);
                    }
                    let mut sends =
                        std::iter::from_fn(|| runtime.try_recv_command()).filter_map(|command| {
                            match command {
                                RuntimeCommand::Prompt {
                                    submission_id,
                                    provisional_title,
                                    ..
                                } => Some((submission_id, provisional_title)),
                                _ => None,
                            }
                        });
                    let (first, _) = sends.next().unwrap();
                    let (second, title) = sends.next().unwrap();
                    let repo = persistence::shared().unwrap();
                    let queued = repo
                        .submit_prompt(PromptSubmission {
                            target: &target,
                            harness: Backend::Codex,
                            project,
                            session: None,
                            profile_id: None,
                            provisional_title: title.as_deref(),
                            mode: PromptMode::Normal,
                            message: "Prompt B",
                            display_message: None,
                            invocation: None,
                            images: &[],
                        })
                        .unwrap();
                    repo.record_session_facts(
                        ObservedSession {
                            harness: Backend::Codex,
                            project,
                            path: &path,
                            profile_id: None,
                            draft: Some(&id),
                        },
                        &SessionFacts::default(),
                    )
                    .unwrap();
                    let row = repo
                        .with(|db| Ok(db.cached_sessions("")?.remove(0)))
                        .unwrap();
                    runtime.send_event(RuntimeEvent::SessionUpdated(row));
                    runtime.send_event(RuntimeEvent::PromptResult {
                        submission_id: Some(first),
                        target: target.clone(),
                        outcome: PromptOutcome::RejectedBeforeAcceptance,
                        session: None,
                    });
                    app.drain_runtime(cx);
                    app.resolve_pending_submission(window, cx);
                    assert_eq!(app.composer.input.read(cx).value().as_ref(), "Prompt A");
                    assert!(app.composer.pending_submissions.contains_key(&second));
                    assert_eq!(
                        app.activity.run_statuses.get(&target),
                        Some(&RunStatus::Working)
                    );
                    assert!(app.editable_draft_project().is_none());
                    runtime.send_event(RuntimeEvent::DraftPromptCommitted {
                        draft: id.clone(),
                        app_session_id: queued.app_session_id,
                        provisional_title: title,
                    });
                    app.drain_runtime(cx);
                    let draft = app
                        .sessions
                        .drafts
                        .iter()
                        .find(|draft| draft.id == id)
                        .unwrap();
                    assert!(draft.submitted);
                    assert_eq!(draft.title.as_deref(), Some("Prompt B"));
                    let saved = repo.with(|db| Ok(db.load_drafts()?.remove(0))).unwrap();
                    assert_eq!(saved.title, draft.title);
                    assert_eq!(saved.session_path.as_ref(), Some(&path));
                    assert_eq!(saved.app_session_id, draft.app_session_id);
                })
            });
        },
    );
}

#[gpui::test]
fn existing_session_submit_keeps_working_until_the_prompt_result(cx: &mut gpui::TestAppContext) {
    use crate::{agents::Backend, runtime::RuntimeEvent};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::existing_session_submit_keeps_working_until_the_prompt_result"
        ),
        cx,
        |cx, app, runtime, project| {
            let path = project.join("existing.jsonl");
            let target = session_target(&path);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    let current = app.composer.sessions.current_target().to_owned();
                    app.composer
                        .sessions
                        .discard_and_switch(&current, target.clone());
                    let snapshot = Arc::make_mut(&mut app.snapshot);
                    snapshot.harness = Some(Backend::Claude);
                    snapshot.selected_session = Some(path.clone());
                    snapshot.history_preview = true;
                    snapshot.live_status = RunStatus::Done;
                    app.record_run_status(target.clone(), RunStatus::Done, true);
                    app.submit("Continue".into(), PromptMode::Normal, window, cx);
                    assert_eq!(app.activity.run_statuses[&target], RunStatus::Working);
                    assert!(!app.activity.recent_completions.contains_key(&target));
                    assert_eq!(app.composer.pending_submissions.len(), 1);
                });
            });
            let submission_id = cx.update(|_, cx| {
                app.read(cx)
                    .composer
                    .pending_submissions
                    .keys()
                    .next()
                    .unwrap()
                    .clone()
            });
            runtime.send_event(RuntimeEvent::SessionStatus {
                record: None,
                target: target.clone(),
                session: Some(path.clone()),
                status: RunStatus::Done,
            });
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert_eq!(app.activity.run_statuses[&target], RunStatus::Working);
                });
            });
            runtime.send_event(RuntimeEvent::PromptResult {
                submission_id: Some(submission_id),
                target: target.clone(),
                outcome: crate::agents::PromptOutcome::Accepted,
                session: Some(path.clone()),
            });
            runtime.send_event(RuntimeEvent::SessionStatus {
                record: None,
                target: target.clone(),
                session: Some(path),
                status: RunStatus::Done,
            });
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert_eq!(app.activity.run_statuses[&target], RunStatus::Done);
                });
            });
        },
    );
}

#[test]
fn archived_sessions_activate_when_their_message_is_sent() {
    let path = Path::new("/sessions/inactive.jsonl");
    let row = session("/sessions/inactive.jsonl", true);
    let key = row.catalog_key();
    let archived = vec![row].into();
    assert_eq!(
        inactive_session_for_target(&session_target(path), Some(path), &archived),
        Some(key)
    );
    assert_eq!(
        inactive_session_for_target("session:/sessions/other.jsonl", Some(path), &archived,),
        None
    );
    let active = vec![session("/sessions/inactive.jsonl", false)].into();
    assert_eq!(
        inactive_session_for_target(&session_target(path), Some(path), &active),
        None
    );
}

#[gpui::test]
fn submitting_to_an_archived_session_restores_only_its_record(cx: &mut gpui::TestAppContext) {
    use crate::{agents::Backend, runtime::ArchiveTarget, sessions::AppSessionId};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::submitting_to_an_archived_session_restores_only_its_record"
        ),
        cx,
        |cx, app, runtime, project| {
            let path = project.join("shared.jsonl");
            let mut selected = session(path.to_str().unwrap(), true).with_app_session_id(31);
            selected.harness = Backend::Codex;
            selected.project = project.into();
            let mut sibling = selected.clone().with_app_session_id(32);
            sibling.harness = Backend::Cursor;
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    let rows = vec![selected.clone(), sibling.clone()];
                    app.sessions.all = rows.clone().into();
                    app.sessions.visible = rows.into();
                    app.switch_composer_target(
                        crate::sessions::session_buffer(selected.harness, &path),
                        window,
                        cx,
                    );
                    let snapshot = Arc::make_mut(&mut app.snapshot);
                    snapshot.project = project.into();
                    snapshot.harness = Some(selected.harness);
                    snapshot.selected_session = Some(path);
                    app.submit("resume this chat".into(), PromptMode::Normal, window, cx);
                    assert!(
                        !app.sessions
                            .all
                            .by_key(&selected.catalog_key())
                            .unwrap()
                            .archived
                    );
                    assert!(
                        app.sessions
                            .all
                            .by_key(&sibling.catalog_key())
                            .unwrap()
                            .archived
                    );
                    let restored =
                        std::iter::from_fn(|| runtime.try_recv_command()).find_map(|command| {
                            match command {
                                RuntimeCommand::SetSessionArchived { request, .. } => {
                                    Some(request.target)
                                }
                                _ => None,
                            }
                        });
                    assert_eq!(
                        restored,
                        Some(ArchiveTarget::App(AppSessionId::new(31).unwrap()))
                    );
                });
            });
        },
    );
}

#[test]
fn rejected_attachment_only_submission_moves_to_its_real_session_after_navigation() {
    let session = Path::new("/sessions/one.jsonl");
    assert_eq!(
        rejected_attachment_target("", true, "draft:one", "session:other", Some(session),),
        Some(session_target(session))
    );
    assert_eq!(
        rejected_attachment_target("typed", true, "draft:one", "session:other", Some(session),),
        None
    );
    assert_eq!(
        rejected_attachment_target("", true, "draft:one", "draft:one", Some(session)),
        None
    );
}

#[test]
fn pending_submission_is_scoped_to_its_own_composer() {
    let pending = std::collections::HashMap::from([("session:compacting".into(), pending())]);

    assert!(has_pending_submission(&pending, "session:compacting"));
    assert!(!has_pending_submission(&pending, "session:other"));
    assert!(!has_pending_submission(&pending, "draft:new"));
}

#[test]
fn unresolved_queue_keeps_submission_order_and_equal_text() {
    let first_at = Instant::now();
    let mut first = pending();
    first.id = "first".into();
    first.submitted_at = first_at;
    first.text = "same text".into();
    first.mode = PromptMode::Steer;
    let mut second = pending();
    second.id = "second".into();
    second.submitted_at = first_at + Duration::from_millis(1);
    second.text = "same text".into();
    second.mode = PromptMode::FollowUp;
    let pending =
        std::collections::HashMap::from([(second.id.clone(), second), (first.id.clone(), first)]);

    let queue = visible_prompt_queue(
        &crate::conversation::QueueState::default(),
        &pending,
        "session:compacting",
    );
    assert_eq!(queue.steering, ["same text"]);
    assert_eq!(queue.follow_up, ["same text"]);
}

#[test]
fn unresolved_submission_overlay_does_not_invent_cancellation_rights() {
    let mut local = pending();
    local.mode = PromptMode::Steer;
    let id = local.id.clone();
    let pending = std::collections::HashMap::from([(id.clone(), local)]);
    let visible = visible_prompt_queue(
        &crate::conversation::QueueState::default(),
        &pending,
        "session:compacting",
    );
    assert_eq!(visible.steering.len(), 1);
    assert!(!visible.can_cancel(&id));
    let runtime_owned = crate::conversation::QueueState {
        cancellable_ids: vec![id.clone()],
        ..Default::default()
    };
    let visible = visible_prompt_queue(&runtime_owned, &pending, "session:compacting");
    assert!(visible.can_cancel(&id));
}

#[test]
fn attachment_only_submissions_are_visible_in_both_queue_modes() {
    let image = ComposerImage {
        prompt: PromptImage::new(String::new(), "image/png".into()),
        preview: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, Vec::new())),
        byte_len: 0,
    };
    for mode in [PromptMode::Steer, PromptMode::FollowUp] {
        let submission = PendingSubmission {
            mode,
            text: String::new(),
            images: vec![image.clone()],
            ..pending()
        };
        let pending = std::collections::HashMap::from([(submission.id.clone(), submission)]);
        let queue = visible_prompt_queue(
            &crate::conversation::QueueState::default(),
            &pending,
            "session:compacting",
        );
        let entries = match mode {
            PromptMode::Steer => queue.steering,
            _ => queue.follow_up,
        };
        assert_eq!(entries, ["1 image"]);
    }
}

#[test]
fn visible_queue_overlays_a_local_submission_already_reflected_by_the_backend() {
    let mut local = pending();
    local.id = "local-steer".into();
    local.text = "same text".into();
    let pending = std::collections::HashMap::from([(local.id.clone(), local)]);
    let native = crate::conversation::QueueState {
        steering: vec!["same text".into()],
        follow_up: vec!["native follow-up".into()],
        ..Default::default()
    };

    let visible = visible_prompt_queue(&native, &pending, "session:compacting");

    assert_eq!(visible.steering, ["same text"]);
    assert_eq!(visible.follow_up, ["native follow-up"]);
}

#[test]
fn visible_queue_matches_the_trimmed_payload_and_keeps_repeated_submissions() {
    let mut local = pending();
    local.text = "  same text\n".into();
    let mut repeated = local.clone();
    repeated.id = "second".into();
    repeated.submitted_at += Duration::from_millis(1);
    let mut pending = std::collections::HashMap::from([(local.id.clone(), local)]);
    let native = crate::conversation::QueueState {
        steering: vec!["same text".into()],
        steering_ids: vec!["backend-steer".into()],
        ..Default::default()
    };

    let visible = visible_prompt_queue(&native, &pending, "session:compacting");
    assert_eq!(visible.steering, ["  same text\n"]);
    assert_eq!(visible.steering_ids, ["backend-steer"]);

    pending.insert(repeated.id.clone(), repeated);
    let visible = visible_prompt_queue(&native, &pending, "session:compacting");
    assert_eq!(visible.steering.len(), 2);
    assert_eq!(visible.steering_ids, ["backend-steer", "second"]);
}

#[test]
fn visible_queue_preserves_the_count_of_repeated_submissions() {
    let first_at = Instant::now();
    let mut first = pending();
    first.id = "first".into();
    first.text = "same text".into();
    first.submitted_at = first_at;
    let mut second = first.clone();
    second.id = "second".into();
    second.submitted_at = first_at + Duration::from_millis(1);
    let pending =
        std::collections::HashMap::from([(first.id.clone(), first), (second.id.clone(), second)]);
    let native = crate::conversation::QueueState {
        steering: vec!["same text".into()],
        follow_up: Vec::new(),
        ..Default::default()
    };

    let visible = visible_prompt_queue(&native, &pending, "session:compacting");

    assert_eq!(visible.steering, ["same text", "same text"]);
}

#[test]
fn visible_queue_uses_local_attachment_preview_for_a_matching_native_item() {
    let mut local = pending();
    local.text.clear();
    local.images.push(image("queued-image"));
    let pending = std::collections::HashMap::from([(local.id.clone(), local)]);
    let native = crate::conversation::QueueState {
        steering: vec![String::new()],
        follow_up: Vec::new(),
        ..Default::default()
    };

    let visible = visible_prompt_queue(&native, &pending, "session:compacting");

    assert_eq!(visible.steering, ["1 image"]);
}

#[test]
fn only_explicit_preacceptance_rejection_restores_the_composer() {
    assert!(restores_composer(
        crate::agents::PromptOutcome::RejectedBeforeAcceptance
    ));
    assert!(!restores_composer(crate::agents::PromptOutcome::Accepted));
    assert!(!restores_composer(crate::agents::PromptOutcome::Cancelled));
    assert!(!restores_composer(
        crate::agents::PromptOutcome::DeliveryUnknown
    ));
}

#[test]
fn terminal_unknown_releases_the_in_memory_submission_to_durable_recovery() {
    let mut submissions = std::collections::HashMap::from([(
        "draft:unknown".into(),
        PendingSubmission {
            result: Some((crate::agents::PromptOutcome::DeliveryUnknown, None)),
            ..pending()
        },
    )]);
    let resolved = take_resolved_pending_submissions(&mut submissions);
    assert!(submissions.is_empty());
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].2, crate::agents::PromptOutcome::DeliveryUnknown);
    assert!(!restores_composer(resolved[0].2));
}

#[test]
fn resolved_submissions_are_ordered_by_submission_time_then_id() {
    let base = Instant::now();
    let resolved_submission = |id: &str, submitted_at: Instant| PendingSubmission {
        id: id.into(),
        submitted_at,
        result: Some((crate::agents::PromptOutcome::RejectedBeforeAcceptance, None)),
        ..pending()
    };
    let mut submissions = std::collections::HashMap::from([
        (
            "late".to_owned(),
            resolved_submission("late", base + Duration::from_millis(2)),
        ),
        (
            "tie-b".to_owned(),
            resolved_submission("tie-b", base + Duration::from_millis(1)),
        ),
        (
            "tie-a".to_owned(),
            resolved_submission("tie-a", base + Duration::from_millis(1)),
        ),
        ("early".to_owned(), resolved_submission("early", base)),
    ]);

    let resolved = take_resolved_pending_submissions(&mut submissions);

    let ids = resolved
        .iter()
        .map(|(_, pending, _, _)| pending.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["early", "tie-a", "tie-b", "late"]);
}

#[test]
fn a_later_failure_waits_for_the_earlier_submission_in_the_same_chat() {
    let base = Instant::now();
    let first = PendingSubmission {
        id: "first".into(),
        submitted_at: base,
        submitted_target: "session:one".into(),
        text: "first".into(),
        result: None,
        ..pending()
    };
    let second = PendingSubmission {
        id: "second".into(),
        submitted_at: base + Duration::from_millis(1),
        submitted_target: "session:one".into(),
        text: "second".into(),
        result: Some((crate::agents::PromptOutcome::RejectedBeforeAcceptance, None)),
        ..pending()
    };
    let mut submissions =
        std::collections::HashMap::from([(first.id.clone(), first), (second.id.clone(), second)]);

    assert!(take_resolved_pending_submissions(&mut submissions).is_empty());
    assert_eq!(submissions.len(), 2);
    submissions
        .get_mut("first")
        .expect("first submission is pending")
        .result = Some((crate::agents::PromptOutcome::RejectedBeforeAcceptance, None));
    let resolved = take_resolved_pending_submissions(&mut submissions);
    assert_eq!(
        resolved
            .iter()
            .map(|(_, row, _, _)| row.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn rejected_submissions_restore_every_text_in_order() {
    use crate::app::composer::sessions::ComposerSessions;

    let target = "session:one";
    let first = PendingSubmission {
        id: "first".into(),
        text: "first".into(),
        ..pending()
    };
    let second = PendingSubmission {
        id: "second".into(),
        text: "second".into(),
        ..pending()
    };
    let mut sessions = ComposerSessions::for_test(target.into());

    let first_restored = restore_rejected_text(&mut sessions, target, &first);
    let second_restored = restore_rejected_text(&mut sessions, target, &second);

    assert!(first_restored.is_some());
    assert!(second_restored.is_some());
    assert_eq!(sessions.snapshot_for(target).text, "first\n\nsecond");
}

#[test]
fn rejected_submissions_keep_an_existing_newer_draft() {
    use crate::app::composer::sessions::ComposerSessions;

    let target = "session:one";
    let first = PendingSubmission {
        id: "first".into(),
        text: "first".into(),
        ..pending()
    };
    let second = PendingSubmission {
        id: "second".into(),
        text: "second".into(),
        ..pending()
    };
    let mut sessions = ComposerSessions::for_test(target.into());
    sessions.capture_current(ComposerSnapshot::new("newer draft".into(), 11, 11..11));

    let _ = restore_rejected_text(&mut sessions, target, &first);
    let _ = restore_rejected_text(&mut sessions, target, &second);

    assert_eq!(
        sessions.snapshot_for(target).text,
        "newer draft\n\nfirst\n\nsecond"
    );
}

#[test]
fn restoring_rejected_text_leaves_composer_history_alone() {
    use crate::app::composer::sessions::ComposerSessions;

    let target = "session:one";
    let mut sessions = ComposerSessions::for_test(target.into());
    sessions.record_submission(target, "sent prompt");
    let rejected = PendingSubmission {
        text: "restored text".into(),
        ..pending()
    };

    let _ = restore_rejected_text(&mut sessions, target, &rejected);

    assert_eq!(
        sessions
            .previous_history(ComposerSnapshot::default())
            .map(|snapshot| snapshot.text),
        Some("sent prompt".into())
    );
}

#[test]
fn submission_delivery_only_treats_registered_names_as_commands() {
    let commands = [crate::protocol::SlashCommand {
        name: "settings".into(),
        description: None,
        source: crate::protocol::SlashCommandSource::Extension,
    }];
    for requested in [PromptMode::Normal, PromptMode::Steer, PromptMode::FollowUp] {
        for message in ["/settings", "  /settings argument"] {
            assert_eq!(
                submission_delivery(message, requested, &commands),
                (PromptMode::Normal, true)
            );
        }
        for message in [
            "ordinary prompt",
            "/tmp/takeout.zip inspect this",
            "  /settings-extra",
        ] {
            assert_eq!(
                submission_delivery(message, requested, &commands),
                (requested, false),
                "{message:?} must retain {requested:?} delivery"
            );
        }
    }
    assert_eq!(
        submission_delivery("/settings", PromptMode::Steer, &[]),
        (PromptMode::Steer, false)
    );
}

#[test]
fn enter_prompts_when_idle_and_steers_while_running() {
    assert_eq!(prompt_mode_for_enter(false), PromptMode::Normal);
    assert_eq!(prompt_mode_for_enter(true), PromptMode::Steer);
}

#[test]
fn tab_prompts_when_idle_and_queues_a_follow_up_while_running() {
    assert_eq!(prompt_mode_for_follow_up(false), PromptMode::Normal);
    assert_eq!(prompt_mode_for_follow_up(true), PromptMode::FollowUp);
}
