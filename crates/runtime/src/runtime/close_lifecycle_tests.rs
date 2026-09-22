//! Exercise native adapters and ensure completed work leaves no runtime lease.
use super::*;
use crate::agents::Backend;

fn assert_close_is_idle(scenario: &mut Scenario, expected_replies: usize) {
    let path = scenario
        .owner
        .active_session
        .clone()
        .expect("active session");
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
                    .with(|store| store.update_session_metadata(&metadata))
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
                let _ = (target, session);
            }
            _ => {}
        }
    }
    assert!(!sessions.is_empty());
    assert_eq!(replies.len(), expected_replies);
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
    assert!(!scenario.owner.normal_prompt_in_flight);
    assert!(scenario.owner.queued_prompts.is_empty());
    assert_eq!(
        scenario.owner.active_session.as_deref(),
        Some(path.as_path())
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
            let target = format!("session:{}", path.display());
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
            assert_close_is_idle(&mut scenario, 1);
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
            let target = format!(
                "session:{}",
                scenario
                    .owner
                    .active_session
                    .as_ref()
                    .expect("active session")
                    .display(),
            );
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
            assert_close_is_idle(&mut scenario, 2);
        },
    );
}
