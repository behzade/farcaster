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
        text: "submitted".into(),
        images: Vec::new(),
        pastes: Vec::new(),
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
fn every_slash_command_uses_backend_prompt_semantics() {
    assert_eq!(
        submission_delivery("/settings", PromptMode::Steer),
        (PromptMode::Normal, true)
    );
    assert_eq!(
        submission_delivery("  /backend-command argument", PromptMode::FollowUp),
        (PromptMode::Normal, true)
    );
    assert_eq!(
        submission_delivery("ordinary prompt", PromptMode::Steer),
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
fn composer_escape_flushes_steer_or_double_taps_to_abort() {
    use ComposerEscapeAction::{Abort, ApplySteering, None as NoAction};
    let t0 = Instant::now();
    let within = t0 + Duration::from_millis(400);
    let expired = t0 + Duration::from_millis(501);
    let one = "session:one";
    let two = "session:two";
    let armed = (one.to_owned(), t0);

    assert_eq!(
        composer_escape(false, true, one, None, t0),
        (NoAction, None)
    );
    assert_eq!(
        composer_escape(false, false, one, Some(&armed), within),
        (NoAction, None)
    );
    assert_eq!(
        composer_escape(true, true, one, Some(&armed), t0),
        (Abort, None)
    );
    assert_eq!(
        composer_escape(true, true, one, None, t0),
        (ApplySteering, Some((one.into(), t0)))
    );
    assert_eq!(
        composer_escape(true, false, one, None, t0),
        (NoAction, Some((one.into(), t0)))
    );
    assert_eq!(
        composer_escape(true, false, one, Some(&armed), within),
        (Abort, None)
    );
    assert_eq!(
        composer_escape(true, false, one, Some(&armed), expired),
        (NoAction, Some((one.into(), expired)))
    );
    assert_eq!(
        composer_escape(true, false, two, Some(&armed), t0),
        (NoAction, Some((two.into(), t0)))
    );
}
