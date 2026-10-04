use crate::agents::Backend;
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use super::*;

#[test]
fn startup_and_later_draft_selection_use_the_same_profile_defaults() -> Result<(), String> {
    let host = crate::test_support::host();
    let state = host.state_store()?;
    let project = PathBuf::from("/profile-startup");
    let profile_id = uuid::Uuid::new_v4().to_string();
    let mut draft = crate::sessions::DraftSession::with_id(
        Some(Backend::Codex),
        "custom-startup".into(),
        project.clone(),
    );
    draft.profile_id = Some(profile_id.clone());
    state.with(|store| store.allocate_app_session_id(&draft))?;
    let defaults = [None, Some(profile_id.clone())].map(|profile_id| {
        let custom = profile_id.is_some();
        farcaster_storage::CachedSessionControlDefaults {
            harness: Backend::Codex,
            profile_id,
            model: Some(
                serde_json::from_value(json!({
                    "id": if custom { "custom-model" } else { "base-model" },
                    "name": "Test model", "provider": "openai", "contextWindow": 1000,
                    "reasoning": true
                }))
                .expect("model"),
            ),
            effort: Some(if custom { "high" } else { "low" }.into()),
            access_mode: Some(if custom {
                HarnessAccessMode::Full
            } else {
                HarnessAccessMode::Auto
            }),
        }
    });
    state.with(|store| store.save_session_control_defaults(&defaults))?;
    let (commands, command_rx) = mpsc::channel();
    let (events, _) = mpsc::channel();
    let (wake, _) = async_channel::bounded(1);
    let mut supervisor = Supervisor::new(
        project.clone(),
        draft,
        None,
        AgentLaunchConfig::default(),
        host,
        command_rx,
        mpsc::channel().1,
        UiEventSender { events, wake },
        false,
    );
    for select_again in [false, true] {
        if select_again {
            commands
                .send(RuntimeCommand::ResumeDraft {
                    id: "custom-startup".into(),
                    harness: Some(Backend::Codex),
                    project: project.clone(),
                })
                .map_err(|error| error.to_string())?;
            assert!(supervisor.process_next_command());
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            supervisor.drain_actor_events();
            if supervisor
                .latest
                .get("draft:custom-startup")
                .is_some_and(|snapshot| {
                    snapshot.profile_id.as_deref() == Some(profile_id.as_str())
                        && snapshot
                            .session_identity()
                            .model
                            .is_some_and(|model| model.id == "custom-model")
                        && snapshot.session_identity().effort == Some("high")
                        && snapshot.access_mode == HarnessAccessMode::Full
                })
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "custom profile defaults missing after selection={select_again}"
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    for actor in supervisor.actors.values() {
        actor.send(RuntimeCommand::Shutdown);
    }
    Ok(())
}

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
    let root = directory
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let initial_project = root.join("initial");
    let pi_project = root.join("pi");
    let codex_project = root.join("codex");
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
        mpsc::channel().1,
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
                .is_some_and(|snapshot| snapshot.prompt_queue().saved.len() == 1)
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
        assert_eq!(snapshot.prompt_queue().saved.len(), 1);
        assert_eq!(snapshot.prompt_queue().saved[0].id, id);
        assert_eq!(snapshot.prompt_queue().saved[0].target, key);
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
                if snapshot.prompt_queue().saved.len() == 1
                    && snapshot.prompt_queue().saved[0].id == id
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
