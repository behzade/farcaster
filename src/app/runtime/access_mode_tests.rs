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
    .unwrap();
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
    .unwrap();
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
