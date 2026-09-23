use super::*;
use crate::agents::Backend;
use crate::runtime::tests::owner_without_process;

#[test]
fn sandbox_changes_show_pending_requested_mode_for_every_native_adapter() {
    use crate::agents::SandboxState;
    use HarnessAccessMode::{Full, Sandboxed};
    for backend in Backend::ALL
        .into_iter()
        .filter(|backend| *backend != Backend::Pi)
    {
        for (current, requested) in [(Full, Sandboxed), (Sandboxed, Full)] {
            let (mut owner, _events) = owner_without_process(std::env::temp_dir());
            owner.harness = Some(backend);
            owner.snapshot.harness = Some(backend);
            owner.process_command.access_mode = current;
            conversation_mut(owner.active_snapshot_mut()).running = true;
            owner.set_access_mode(requested);
            assert_eq!(owner.snapshot.access_mode, requested, "{backend:?}");
            assert_eq!(
                owner.snapshot.sandbox_state,
                SandboxState::Pending(current),
                "{backend:?}"
            );
            assert_eq!(owner.process_command.access_mode, current);
            owner.access_mode_changes.make_due();
            owner.apply_queued_access_mode_change();
            assert_eq!(owner.snapshot.sandbox_state, SandboxState::Pending(current));
            // Selecting the effective mode again cancels the transition.
            owner.set_access_mode(current);
            assert_eq!(owner.snapshot.sandbox_state, SandboxState::Active(current));
            assert!(owner.access_mode_changes.is_idle());
        }
    }
}

#[test]
fn access_modes_reject_unsupported_and_recheck_queued_changes() {
    use HarnessAccessMode::{Auto, Full, Sandboxed};
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.harness = Some(Backend::Claude);
    owner.snapshot.harness = Some(Backend::Claude);
    owner.process_command.access_mode = Sandboxed;
    owner.set_access_mode(Auto);
    assert_eq!(owner.process_command.access_mode, Sandboxed);
    let model: Model = serde_json::from_value(serde_json::json!({
        "id":"model", "name":"Model", "provider":"claude",
        "access_modes":["sandboxed", "auto", "full"]
    }))
    .expect("test operation should succeed");
    owner.snapshot.models = vec![model];
    conversation_mut(owner.active_snapshot_mut()).running = true;
    owner.set_access_mode(Auto);
    assert_eq!(owner.snapshot.access_mode, Auto);
    owner.snapshot.models[0].access_modes = Some(vec![Sandboxed, Full]);
    conversation_mut(owner.active_snapshot_mut()).running = false;
    owner.access_mode_changes.make_due();
    owner.apply_queued_access_mode_change();
    assert_eq!(owner.snapshot.access_mode, Sandboxed);
    assert_eq!(owner.process_command.access_mode, Sandboxed);
    assert!(owner.access_mode_changes.is_idle());
}

#[test]
fn access_modes_queue_safe_mode_before_switching_an_auto_session_model() {
    struct NoCommands;
    impl SessionTransport for NoCommands {
        fn send(&mut self, _: SessionCommand) -> Result<String, String> {
            panic!("an unsupported model change must not reach the backend")
        }
        fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<SessionEvent> {
            None
        }
        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.harness = Some(Backend::Claude);
    owner.process_command.access_mode = HarnessAccessMode::Auto;
    owner.process = Some(Box::new(NoCommands));
    let model: Model = serde_json::from_value(json!({
        "id":"limited", "name":"Limited", "provider":"claude",
        "access_modes":["sandboxed", "full"]
    }))
    .expect("test operation should succeed");
    owner.set_model(model.clone());
    assert!(owner.pending_session_controls.model_pending());
    assert_eq!(owner.snapshot.prefill_model, Some(model.clone()));
    assert_eq!(owner.process_command.access_mode, HarnessAccessMode::Auto);
    assert_eq!(owner.snapshot.access_mode, HarnessAccessMode::Sandboxed);
    owner.process = None;
    owner.snapshot.selected_session = None;
    owner.set_model(model.clone());
    assert_eq!(owner.snapshot.prefill_model, Some(model));
    assert_eq!(
        owner.process_command.access_mode,
        HarnessAccessMode::Sandboxed
    );
}

#[test]
fn chosen_full_mode_queues_model_and_effort_until_restart() {
    struct NoCommands;
    impl SessionTransport for NoCommands {
        fn send(&mut self, _: SessionCommand) -> Result<String, String> {
            panic!("model and effort must wait for the mode change")
        }
        fn respond(&mut self, _: ExtensionUiResponse) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<SessionEvent> {
            None
        }
        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
    let (mut owner, _) = owner_without_process(std::env::temp_dir());
    owner.harness = Some(Backend::Claude);
    owner.process_command.access_mode = HarnessAccessMode::Sandboxed;
    owner.process = Some(Box::new(NoCommands));
    let model: Model = serde_json::from_value(json!({
        "id":"full-only", "name":"Full only", "provider":"claude",
        "access_modes":["full"]
    }))
    .expect("decode model");

    owner.set_model_with_access_mode(model.clone(), HarnessAccessMode::Full);
    owner.set_thinking("high".into());

    assert_eq!(owner.snapshot.access_mode, HarnessAccessMode::Full);
    assert_eq!(
        owner.process_command.access_mode,
        HarnessAccessMode::Sandboxed
    );
    assert_eq!(owner.snapshot.prefill_model, Some(model));
    assert!(owner.pending_session_controls.model_pending());
    assert!(!owner.pending_session_controls.is_empty());
}

#[test]
fn chosen_mode_overrides_a_stale_fallback_for_an_unstarted_session() {
    let (mut owner, _) = owner_without_process(std::env::temp_dir());
    owner.harness = Some(Backend::Claude);
    owner.snapshot.harness = Some(Backend::Claude);
    owner.process_command.access_mode = HarnessAccessMode::Auto;
    let model: Model = serde_json::from_value(json!({
        "id":"full-only", "name":"Full only", "provider":"claude",
        "access_modes":["full"]
    }))
    .expect("decode model");
    assert_eq!(
        owner
            .access_mode_changes
            .resolve_available(HarnessAccessMode::Auto, &[HarnessAccessMode::Sandboxed],),
        Some(HarnessAccessMode::Sandboxed)
    );

    owner.set_model_with_access_mode(model, HarnessAccessMode::Full);

    assert_eq!(owner.process_command.access_mode, HarnessAccessMode::Full);
    assert_eq!(owner.snapshot.access_mode, HarnessAccessMode::Full);
}

#[test]
fn unsupported_harness_never_promotes_auto_to_full() {
    use HarnessAccessMode::{Auto, Sandboxed};
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.harness = None;
    owner.process_command.access_mode = Auto;
    owner.publish();
    assert_eq!(owner.process_command.access_mode, Auto);
    assert!(owner.snapshot.available_access_modes().is_empty());
    owner.stage_draft(Some(Backend::Codex), std::env::temp_dir());
    assert_eq!(owner.snapshot.access_mode, Auto);
    assert_eq!(owner.process_command.access_mode, Auto);
    owner.set_access_mode(Sandboxed);
    owner.publish();
    assert_eq!(owner.snapshot.access_mode, Sandboxed);
}

#[test]
fn sandbox_discovery_rechecks_every_restart_and_mode_changes_wait_for_idle()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::agents::SandboxState;
    use crate::runtime::tests::drive_process_until;
    let temp = tempfile::tempdir()?;
    let script = temp.path().join("pi.sh");
    std::fs::write(
        &script,
        include_str!("../../../../tests/fixtures/fake-pi.sh"),
    )?;
    let (mut owner, _events) = owner_without_process(temp.path().to_owned());
    owner.process_command = AgentLaunchConfig::test_script(&script, vec!["sandbox-ready".into()]);
    owner.start_process(Some(temp.path().join("session.jsonl")));
    drive_process_until(&mut owner, |owner| {
        owner.startup_state_loaded && owner.startup_history_loaded
    });
    assert_eq!(
        owner.snapshot.sandbox_state,
        SandboxState::Active(HarnessAccessMode::Sandboxed)
    );
    let generation = owner.process_generation;
    conversation_mut(owner.active_snapshot_mut()).running = true;
    owner.set_access_mode(HarnessAccessMode::Full);
    assert_eq!(
        owner.snapshot.sandbox_state,
        SandboxState::Pending(HarnessAccessMode::Sandboxed)
    );
    owner.access_mode_changes.make_due();
    owner.apply_queued_access_mode_change();
    assert_eq!(owner.process_generation, generation);
    conversation_mut(owner.active_snapshot_mut()).running = false;
    owner.apply_queued_access_mode_change();
    drive_process_until(&mut owner, |owner| {
        owner.startup_state_loaded && owner.startup_history_loaded
    });
    assert_eq!(
        owner.snapshot.sandbox_state,
        SandboxState::Active(HarnessAccessMode::Full)
    );
    assert_eq!(owner.process_generation, generation + 1);
    owner.set_access_mode(HarnessAccessMode::Sandboxed);
    owner.process_command.prefix_args[1] = "sandbox-failed".into();
    owner.access_mode_changes.make_due();
    owner.apply_queued_access_mode_change();
    assert!(owner.process.is_none());
    assert_eq!(owner.snapshot.sandbox_state, SandboxState::Failed);
    assert!(!temp.path().join("agent-prompts").exists());
    Ok(())
}

#[test]
fn cold_catalog_adapter_is_active_until_apply_starts() {
    use crate::agents::SandboxState;
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.snapshot.sandbox_adapter = Some("pi-nono".into());
    owner.publish();
    assert!(owner.process.is_none());
    assert!(owner.snapshot.sandbox_controls_available());
    assert_eq!(
        owner.snapshot.sandbox_state,
        SandboxState::Active(HarnessAccessMode::Sandboxed)
    );

    owner.access_mode_changes.applying = true;
    owner.publish();
    assert_eq!(owner.snapshot.sandbox_state, SandboxState::Checking);
}

#[test]
fn saved_session_access_mode_is_restored_after_runtime_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("state.sqlite3");
    let session = temp.path().join("session-locators/codex-cli/session-1");
    let mut state = StateStore::open_at(&database)?;
    state.update_session_metadata(&crate::agents::SessionMetadata {
        harness: Backend::Codex,
        id: "session-1".into(),
        path: session.clone(),
        project: temp.path().to_owned(),
        title: None,
        first_user_message: None,
        parent_session: None,
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: Some(HarnessAccessMode::Full),
        usage: None,
        is_running: false,
    })?;
    drop(state);

    let (mut owner, _events) = owner_without_process(temp.path().to_owned());
    owner.state = Some(SharedStateStore::open_at(&database)?);
    owner.harness = Some(Backend::Codex);
    owner.snapshot.harness = Some(Backend::Codex);
    owner.process_command.access_mode = HarnessAccessMode::Sandboxed;
    owner.snapshot.access_mode = HarnessAccessMode::Sandboxed;
    let restored = owner
        .state
        .as_ref()
        .expect("state store")
        .with(|store| store.session_access_mode(&session))?
        .expect("saved access mode");
    owner.apply_command(RuntimeCommand::RestoreAccessMode(restored));

    assert_eq!(owner.process_command.access_mode, HarnessAccessMode::Full);
    assert_eq!(owner.snapshot.access_mode, HarnessAccessMode::Full);
    Ok(())
}
