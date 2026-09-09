use super::*;
use std::sync::mpsc;

#[test]
fn queued_search_edits_only_load_the_latest_query() {
    let (sender, receiver) = mpsc::channel();
    for query in ["c", "co", "cod", "code", ""] {
        sender
            .send(RuntimeCommand::LoadSessions(query.into()))
            .unwrap();
    }
    let mut pending = None;
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Ok(RuntimeCommand::LoadSessions(query)) if query.is_empty()
    ));
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Err(TryRecvError::Empty)
    ));
}

#[test]
fn search_coalescing_preserves_other_commands_and_disconnect_order() {
    let (sender, receiver) = mpsc::channel();
    for command in [
        RuntimeCommand::LoadSessions("old".into()),
        RuntimeCommand::RefreshSessions,
        RuntimeCommand::LoadSessions("n".into()),
        RuntimeCommand::LoadSessions("new".into()),
        RuntimeCommand::Shutdown,
    ] {
        sender.send(command).unwrap();
    }
    drop(sender);
    let mut pending = None;
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Ok(RuntimeCommand::LoadSessions(query)) if query == "old"
    ));
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Ok(RuntimeCommand::RefreshSessions)
    ));
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Ok(RuntimeCommand::LoadSessions(query)) if query == "new"
    ));
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Ok(RuntimeCommand::Shutdown)
    ));
    assert!(matches!(
        receive_command(&receiver, &mut pending),
        Err(TryRecvError::Disconnected)
    ));
}
