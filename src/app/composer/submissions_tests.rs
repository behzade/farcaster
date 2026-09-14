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

    let queue = pending_prompt_queue(&pending, "session:compacting");
    assert_eq!(queue.steering, ["same text"]);
    assert_eq!(queue.follow_up, ["same text"]);
}

#[test]
fn visible_queue_is_the_native_queue_plus_each_local_submission() {
    let mut local = pending();
    local.id = "local-steer".into();
    local.text = "same text".into();
    let pending = std::collections::HashMap::from([(local.id.clone(), local)]);
    let native = crate::conversation::QueueState {
        steering: vec!["same text".into()],
        follow_up: vec!["native follow-up".into()],
    };

    let visible = visible_prompt_queue(&native, &pending, "session:compacting");

    assert_eq!(visible.steering, ["same text", "same text"]);
    assert_eq!(visible.follow_up, ["native follow-up"]);
}

#[test]
fn only_explicit_preacceptance_rejection_restores_the_composer() {
    assert!(restores_composer(
        crate::agents::PromptOutcome::RejectedBeforeAcceptance
    ));
    assert!(!restores_composer(crate::agents::PromptOutcome::Accepted));
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
