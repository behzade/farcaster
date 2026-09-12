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
        submitted_target: "session:compacting".into(),
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
fn pending_submission_only_blocks_its_own_composer() {
    let pending = std::collections::HashMap::from([("session:compacting".into(), pending())]);

    assert!(!can_submit_to(&pending, "session:compacting"));
    assert!(can_submit_to(&pending, "session:other"));
    assert!(can_submit_to(&pending, "draft:new"));
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
fn real_gpui_input_routes_distinct_escape_to_raw_dispatch_and_drops_repeat(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{
        AppContext as _, Focusable as _, InputEvent as _, InteractiveElement as _, IntoElement,
        ParentElement as _, Render,
    };
    use gpui_component::input::{Textarea, TextareaState};

    struct EscapeHarness {
        input: gpui::Entity<TextareaState>,
        armed: Option<(String, Instant)>,
        running: bool,
        pending: bool,
        now: Instant,
        actions: Vec<ComposerEscapeAction>,
    }

    impl Render for EscapeHarness {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            gpui::div()
                .capture_key_down(cx.listener(|this, event, window, cx| {
                    match composer_escape_key(event) {
                        ComposerEscapeKeyAction::Ignore => return,
                        ComposerEscapeKeyAction::Consume => {}
                        ComposerEscapeKeyAction::Dispatch => {
                            let (action, armed) = composer_escape(
                                this.running,
                                false,
                                false,
                                this.pending,
                                "session:one",
                                this.armed.as_ref(),
                                this.now,
                            );
                            this.armed = armed;
                            this.running = false;
                            this.pending = false;
                            this.actions.push(action);
                        }
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                }))
                .child(
                    gpui::div()
                        .key_context("FarcasterComposer")
                        .child(Textarea::new(&self.input)),
                )
        }
    }

    cx.update(|cx| {
        gpui_component::init(cx);
        cx.bind_keys(crate::app::ui::keybindings::bindings());
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let input = cx.new(|cx| TextareaState::new(window, cx));
        input.read(cx).focus_handle(cx).focus(window, cx);
        EscapeHarness {
            input,
            armed: None,
            running: true,
            pending: true,
            now: Instant::now(),
            actions: Vec::new(),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));

    cx.simulate_keystrokes("escape");
    let armed = cx.update(|_, cx| {
        assert_eq!(view.read(cx).actions, [ComposerEscapeAction::ApplySteering]);
        view.read(cx)
            .armed
            .clone()
            .expect("first Escape arms abort")
    });

    cx.update(|window, cx| {
        window.dispatch_event(
            gpui::KeyDownEvent {
                keystroke: gpui::Keystroke::parse("escape").expect("test keystroke"),
                is_held: true,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
    });
    cx.update(|_, cx| {
        assert_eq!(view.read(cx).actions, [ComposerEscapeAction::ApplySteering]);
        assert_eq!(view.read(cx).armed.as_ref(), Some(&armed));
    });

    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).actions,
            [
                ComposerEscapeAction::ApplySteering,
                ComposerEscapeAction::Abort
            ]
        );
        assert!(view.read(cx).armed.is_none());
    });
}
