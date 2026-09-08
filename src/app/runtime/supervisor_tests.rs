use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use super::*;

#[test]
fn session_actor_publishes_the_harness_it_was_born_with() {
    let actor = SessionRuntimeHandle::spawn(
        PathBuf::from("/project"),
        AgentLaunchConfig::default(),
        false,
        "cursor-cli".into(),
        thread::current(),
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut harness = None;
    while Instant::now() < deadline {
        while let Ok(event) = actor.events.try_recv() {
            if let RuntimeEvent::Snapshot { snapshot, .. } = event {
                harness = Some(snapshot.harness.clone());
            }
        }
        if harness.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    actor.send(RuntimeCommand::Shutdown);
    assert_eq!(harness.as_deref(), Some("cursor-cli"));
}
