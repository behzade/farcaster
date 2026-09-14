//! Exercise native adapters and replay their results through the UI's real
//! submission reducer and close guards.
use super::*;
use crate::agents::Backend;
use crate::app::{
    composer::{
        sessions::session_target,
        submissions::{PendingSubmission, take_resolved_pending_submissions},
    },
    event_projection::record_pending_prompt_result_for_submission,
    session::activity::{application_has_active_work, session_family_has_active_work},
};

fn pending(id: &str, target: &str) -> PendingSubmission {
    PendingSubmission {
        id: id.into(),
        submitted_at: Instant::now(),
        submitted_target: target.into(),
        mode: PromptMode::Normal,
        text: "Inspect this archive".into(),
        images: Vec::new(),
        pastes: Vec::new(),
        append_on_failure: false,
        result: None,
    }
}

fn assert_close_is_idle(scenario: &mut Scenario, mut pending: HashMap<String, PendingSubmission>) {
    let path = scenario
        .owner
        .active_session
        .clone()
        .expect("active session");
    let target = session_target(&path);
    let mut sessions = Vec::new();
    let mut replies = Vec::new();
    for event in scenario.events.drain(..) {
        match event {
            RuntimeEvent::SessionMetadata(metadata) => {
                let row = scenario
                    .owner
                    .state
                    .as_mut()
                    .expect("state store")
                    .update_session_metadata(&metadata)
                    .expect("save session metadata");
                sessions.retain(|session: &SessionSummary| session.path != row.path);
                sessions.push(row);
            }
            RuntimeEvent::PromptResult {
                submission_id,
                target,
                outcome,
                session,
            } => {
                replies.push((submission_id.clone(), outcome));
                record_pending_prompt_result_for_submission(
                    &mut pending,
                    submission_id.as_deref(),
                    &target,
                    outcome,
                    session,
                );
                take_resolved_pending_submissions(&mut pending);
            }
            _ => {}
        }
    }
    assert!(!sessions.is_empty());
    assert!(!replies.is_empty());
    assert!(
        replies
            .iter()
            .all(|(_, outcome)| *outcome == crate::agents::PromptOutcome::Accepted),
        "fixture must complete accepted prompts: {replies:?}"
    );
    assert!(sessions.iter().all(|session| !session.is_running));
    let snapshot = scenario.owner.active_snapshot();
    assert!(snapshot.conversation.settled);
    assert!(!snapshot.conversation.running);
    let statuses = HashMap::from([(target, semantic_status(snapshot).into())]);
    assert!(
        !session_family_has_active_work(&sessions, &path, &statuses, snapshot, &pending),
        "completed chat still warns; unresolved submissions: {:?}; actual runtime replies: {:?}",
        pending.keys().collect::<Vec<_>>(),
        replies
    );
    assert!(!application_has_active_work(
        &statuses,
        snapshot,
        &pending,
        &sessions,
        &[]
    ));
    assert!(
        pending.is_empty(),
        "completion must resolve submissions, not just suppress the warning"
    );
}

#[test]
fn completed_send_to_chat_releases_close_guard() {
    isolated_title(
        "close_lifecycle_tests::completed_send_to_chat_releases_close_guard",
        || {
            let mut scenario = Scenario::new(Backend::Cursor, Some("Existing chat"), false);
            let path = scenario
                .owner
                .active_session
                .clone()
                .expect("active session");
            let target = session_target(&path);
            let pending =
                HashMap::from([("ui-submission".into(), pending("ui-submission", &target))]);
            scenario.owner.apply_command(RuntimeCommand::SendToSession {
                submission_id: "ui-submission".into(),
                target,
                session: None,
                project: scenario.owner.project.clone(),
                message: "Inspect this archive".into(),
            });
            scenario.until(|s| {
                s.owner.active_snapshot().conversation.settled
                    && !s.owner.active_snapshot().conversation.running
                    && !s.owner.normal_prompt_in_flight
                    && s.events
                        .iter()
                        .any(|event| matches!(event, RuntimeEvent::PromptResult { .. }))
            });
            assert_close_is_idle(&mut scenario, pending);
        },
    );
}

#[test]
fn completed_cold_prompt_and_follow_up_release_close_guard() {
    isolated_title(
        "close_lifecycle_tests::completed_cold_prompt_and_follow_up_release_close_guard",
        || {
            let mut scenario = Scenario::new(Backend::Codex, Some("Existing chat"), true);
            scenario
                .owner
                .process
                .take()
                .expect("active process")
                .close()
                .expect("close process");
            let target = session_target(
                scenario
                    .owner
                    .active_session
                    .as_ref()
                    .expect("active session"),
            );
            let pending = HashMap::from([
                ("first".into(), pending("first", &target)),
                ("second".into(), pending("second", &target)),
            ]);
            for (id, mode) in [
                ("first", PromptMode::Normal),
                ("second", PromptMode::FollowUp),
            ] {
                scenario.owner.apply_command(RuntimeCommand::Prompt {
                    submission_id: id.into(),
                    target: target.clone(),
                    mode,
                    message: "Inspect this archive".into(),
                    display_message: None,
                    invocation: None,
                    images: Vec::new(),
                    allow_while_running: false,
                });
            }
            scenario.until(|s| {
                s.owner.active_snapshot().conversation.settled
                    && !s.owner.active_snapshot().conversation.running
                    && !s.owner.normal_prompt_in_flight
                    && s.owner.queued_prompts.is_empty()
                    && s.events
                        .iter()
                        .filter(|event| matches!(event, RuntimeEvent::PromptResult { .. }))
                        .count()
                        == 2
            });
            assert_close_is_idle(&mut scenario, pending);
        },
    );
}
