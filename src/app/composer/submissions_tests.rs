use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::sessions::UsageSummary;

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

fn image(data: &str) -> ComposerImage {
    ComposerImage {
        prompt: PromptImage::new(data.into(), "image/png".into()),
        preview: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, Vec::new())),
        byte_len: 0,
    }
}

#[test]
fn archived_sessions_activate_when_their_message_is_sent() {
    let path = Path::new("/sessions/inactive.jsonl");
    let archived = [session("/sessions/inactive.jsonl", true)];
    assert_eq!(
        inactive_session_for_target(&session_target(path), Some(path), &archived),
        Some(path.to_path_buf())
    );
    assert_eq!(
        inactive_session_for_target("session:/sessions/other.jsonl", Some(path), &archived,),
        None
    );
    let active = [session("/sessions/inactive.jsonl", false)];
    assert_eq!(
        inactive_session_for_target(&session_target(path), Some(path), &active),
        None
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

    // If restoring rewrote history, the newest entry would be "restored text".
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

#[test]
fn first_escape_applies_pending_submission_before_its_ack() {
    use ComposerEscapeAction::ApplySteering;
    let now = Instant::now();
    assert_eq!(
        composer_escape(false, false, false, true, "session:one", None, now),
        (ApplySteering, Some(("session:one".into(), now)))
    );
}

#[test]
fn first_escape_applies_steering_and_follow_up_queues_immediately() {
    use ComposerEscapeAction::ApplySteering;
    let now = Instant::now();
    for (steering, follow_up) in [(true, false), (false, true)] {
        assert_eq!(
            composer_escape(false, steering, follow_up, false, "session:one", None, now,),
            (ApplySteering, Some(("session:one".into(), now)))
        );
    }
}

#[test]
fn second_escape_aborts_even_if_the_run_looks_idle_between_presses() {
    use ComposerEscapeAction::{Abort, None as NoAction};
    let first = Instant::now();
    let armed = ("session:one".to_owned(), first);
    assert_eq!(
        composer_escape(true, false, false, false, "session:one", None, first),
        (NoAction, Some(armed.clone()))
    );
    assert_eq!(
        composer_escape(
            false,
            false,
            false,
            false,
            "session:one",
            Some(&armed),
            first + Duration::from_millis(400),
        ),
        (Abort, None)
    );
}

#[test]
fn escape_abort_arm_expires_and_is_scoped_to_one_target() {
    use ComposerEscapeAction::{ApplySteering, None as NoAction};
    let t0 = Instant::now();
    let expired = t0 + Duration::from_millis(501);
    let one = "session:one";
    let two = "session:two";
    let armed = (one.to_owned(), t0);

    assert_eq!(
        composer_escape(true, false, false, false, one, Some(&armed), expired),
        (NoAction, Some((one.into(), expired)))
    );
    assert_eq!(
        composer_escape(true, true, false, false, two, Some(&armed), t0),
        (ApplySteering, Some((two.into(), t0)))
    );
}

#[test]
fn composer_escape_is_owned_by_raw_events_and_held_input_never_dispatches() {
    use ComposerEscapeKeyAction::{Consume, Dispatch, Ignore};
    use gpui::Action as _;
    let event = |key: &str, is_held| gpui::KeyDownEvent {
        keystroke: gpui::Keystroke::parse(key).expect("test keystroke"),
        is_held,
        prefer_character_input: false,
    };

    assert_eq!(composer_escape_key(&event("escape", true)), Consume);
    assert_eq!(composer_escape_key(&event("escape", false)), Dispatch);
    assert_eq!(composer_escape_key(&event("cmd-escape", true)), Ignore);
    assert_eq!(composer_escape_key(&event("enter", true)), Ignore);

    let shortcuts = crate::app::ui::keybindings::registry();
    let shortcut = shortcuts
        .iter()
        .find(|shortcut| shortcut.section == "Composer" && shortcut.keystroke == "escape")
        .expect("composer Escape help entry");
    assert!(shortcut.show_in_help);
    assert!(
        shortcut
            .binding
            .action()
            .as_any()
            .downcast_ref::<gpui::Unbind>()
            .is_some_and(|unbind| unbind.0.as_ref() == crate::app::ComposerEscape.name())
    );
    let keymap = gpui::Keymap::new(
        shortcuts
            .into_iter()
            .map(|shortcut| shortcut.binding)
            .collect(),
    );
    let contexts = ["FarcasterComposer", "Input"]
        .map(|context| gpui::KeyContext::parse(context).expect("test key context"));
    assert!(
        keymap
            .bindings_for_input(
                &[gpui::Keystroke::parse("escape").expect("test keystroke")],
                &contexts,
            )
            .0
            .is_empty(),
        "Escape must reach the raw key callback without action dispatch"
    );
}

#[gpui::test]
fn real_app_escape_route_obeys_surface_overlay_focus_and_repeat_gates(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::real_app_escape_route_obeys_surface_overlay_focus_and_repeat_gates"
        ),
        cx,
        |cx, app, runtime, _| {
            let event = |is_held| gpui::KeyDownEvent {
                keystroke: gpui::Keystroke::parse("escape").expect("test keystroke"),
                is_held,
                prefer_character_input: false,
            };
            let route = |cx: &mut gpui::VisualTestContext,
                         surface: crate::app::AppSurface,
                         modal: bool,
                         composer_focused: bool,
                         is_held| {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.workspace.surface = surface;
                        app.composer.escape_armed = None;
                        let mut conversation = crate::conversation::ConversationState::default();
                        conversation.running = true;
                        conversation.queue.steering.push("pending steer".into());
                        Arc::make_mut(&mut app.snapshot).conversation = Arc::new(conversation);
                        app.extensions.active = Default::default();
                        if modal {
                            app.extensions.active.apply(
                                crate::protocol::ExtensionUiRequest::Input {
                                    id: "modal".into(),
                                    title: "Modal".into(),
                                    placeholder: None,
                                    timeout: None,
                                },
                            );
                        }
                        if composer_focused {
                            app.composer.focus.focus(window, cx);
                        } else {
                            app.navigation.search_focus.focus(window, cx);
                        }
                        let consumed = app.handle_composer_escape_key(&event(is_held), window, cx);
                        (consumed, app.composer.escape_armed.is_some())
                    })
                })
            };

            assert_eq!(
                route(cx, crate::app::AppSurface::Chat, false, true, false),
                (true, true),
                "focused Chat must dispatch Escape and arm Abort"
            );
            assert!(matches!(
                runtime.try_recv_command(),
                Some(RuntimeCommand::ApplySteering)
            ));
            assert_eq!(
                route(cx, crate::app::AppSurface::Chat, false, true, true),
                (true, false),
                "held Escape must be consumed without dispatch"
            );
            assert!(runtime.try_recv_command().is_none());
            for surface in [
                crate::app::AppSurface::Editor,
                crate::app::AppSurface::Terminal,
            ] {
                assert_eq!(route(cx, surface, false, true, false), (false, false));
            }
            assert_eq!(
                route(cx, crate::app::AppSurface::Chat, true, true, false),
                (false, false),
                "a modal owns Escape"
            );
            assert_eq!(
                route(cx, crate::app::AppSurface::Chat, false, false, false),
                (false, false),
                "other app focus must not dispatch composer Escape"
            );
            assert!(runtime.try_recv_command().is_none());
        },
    );
}
