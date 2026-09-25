use crate::agents::Backend;
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
        Some(Backend::Cursor),
        thread::current(),
        crate::test_support::host(),
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut harness = None;
    while Instant::now() < deadline {
        while let Ok(event) = actor.events.try_recv() {
            if let RuntimeEvent::Snapshot { snapshot, .. } = event {
                harness = Some(snapshot.harness);
            }
        }
        if harness.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    actor.send(RuntimeCommand::Shutdown);
    assert_eq!(harness.flatten(), Some(Backend::Cursor));
}

#[test]
fn recovered_draft_prompts_follow_their_own_actors_and_stay_saved() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let initial_project = directory.path().join("initial");
    let pi_project = directory.path().join("pi");
    let codex_project = directory.path().join("codex");
    let host = crate::test_support::host();
    let state = host.state_store()?;
    let pi_id = state.with(|store| {
        store.enqueue_prompt(
            "draft:other-pi",
            Backend::Pi,
            &pi_project,
            None,
            PromptMode::Normal,
            "saved for pi",
            &[],
        )
    })?;
    let codex_id = state.with(|store| {
        store.enqueue_prompt(
            "draft:other-codex",
            Backend::Codex,
            &codex_project,
            None,
            PromptMode::Normal,
            "saved for codex",
            &[],
        )
    })?;
    let (commands, command_rx) = mpsc::channel();
    let (events_tx, events) = mpsc::channel();
    let (wake, _) = async_channel::bounded(1);
    let mut supervisor = Supervisor::new(
        initial_project.clone(),
        crate::sessions::DraftSession::with_id(None, "initial".into(), initial_project),
        None,
        AgentLaunchConfig::default(),
        host,
        command_rx,
        UiEventSender {
            events: events_tx,
            wake,
        },
        false,
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        supervisor.drain_actor_events();
        if ["draft:other-pi", "draft:other-codex"].iter().all(|key| {
            supervisor
                .latest
                .get(*key)
                .is_some_and(|snapshot| snapshot.conversation.queue.saved.len() == 1)
        }) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    for (key, harness, project, id) in [
        ("draft:other-pi", Backend::Pi, pi_project, pi_id),
        ("draft:other-codex", Backend::Codex, codex_project, codex_id),
    ] {
        let snapshot = supervisor.latest.get(key).ok_or(format!("missing {key}"))?;
        assert_eq!(snapshot.harness, Some(harness));
        assert_eq!(snapshot.project, project);
        assert_eq!(snapshot.conversation.queue.saved.len(), 1);
        assert_eq!(snapshot.conversation.queue.saved[0].id, id);
        assert_eq!(snapshot.conversation.queue.saved[0].target, key);
        assert!(
            snapshot.live_session.is_none(),
            "recovery must not replay input"
        );

        commands
            .send(RuntimeCommand::ResumeDraft {
                id: key.trim_start_matches("draft:").into(),
                harness: Some(harness),
                project,
            })
            .map_err(|error| error.to_string())?;
        assert!(supervisor.process_next_command());
        assert!(events.try_iter().any(|event| matches!(
            event,
            RuntimeEvent::Snapshot { snapshot, .. }
                if snapshot.conversation.queue.saved.len() == 1
                    && snapshot.conversation.queue.saved[0].id == id
        )));
    }
    assert_eq!(state.with(|store| store.queued_prompts())?.len(), 2);
    for actor in supervisor.actors.values() {
        actor.send(RuntimeCommand::Shutdown);
    }
    for actor in supervisor.actors.into_values() {
        actor.join()?;
    }
    Ok(())
}
