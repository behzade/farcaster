use super::*;

#[test]
fn dismissing_trust_cancels_the_pending_project_command() {
    let mut pending = Some(RuntimeCommand::Shutdown);
    assert!(cancel_pending_command(&mut pending));
    assert!(pending.is_none());
}

#[test]
fn selecting_after_a_new_trust_decision_restarts_the_project_process() {
    let path = PathBuf::from("/session.jsonl");
    let project = PathBuf::from("/project");
    assert!(matches!(
        restart_session_after_trust(RuntimeCommand::SelectSession {
            session_id: path.to_string_lossy().into_owned(),
            path,
            harness: "pi".into(),
            project,
        }),
        RuntimeCommand::RestartSession { .. }
    ));
}
