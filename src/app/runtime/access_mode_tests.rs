use super::*;
use crate::runtime::tests::owner_without_process;

#[test]
fn access_modes_reject_unsupported_and_recheck_queued_changes() {
    use HarnessAccessMode::{Auto, Full, Sandboxed};
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.harness = "claude".into();
    owner.snapshot.harness = "claude".into();
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
fn access_modes_prevent_switching_an_auto_session_to_an_unsupported_model() {
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
    owner.harness = "claude".into();
    owner.process_command.access_mode = HarnessAccessMode::Auto;
    owner.process = Some(Box::new(NoCommands));
    let model: Model = serde_json::from_value(json!({
        "id":"limited", "name":"Limited", "provider":"claude",
        "access_modes":["sandboxed", "full"]
    }))
    .expect("test operation should succeed");
    owner.set_model(model.clone());
    assert!(owner.pending_session_controls.is_empty());
    assert!(owner.snapshot.prefill_model.is_none());
    assert_eq!(owner.process_command.access_mode, HarnessAccessMode::Auto);
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
fn access_modes_restore_auto_after_an_unsupported_harness() {
    use HarnessAccessMode::{Auto, Full, Sandboxed};
    let (mut owner, _events) = owner_without_process(std::env::temp_dir());
    owner.harness = "custom".into();
    owner.process_command.access_mode = Auto;
    owner.publish();
    assert_eq!(owner.process_command.access_mode, Full);
    owner.stage_draft("codex-cli".into(), std::env::temp_dir());
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
    std::fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))?;
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
    assert_eq!(owner.snapshot.sandbox_state, SandboxState::Checking);
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
