use super::*;

#[test]
fn backend_ids_are_stable_machine_keys() -> Result<(), String> {
    assert_eq!(AgentBackendId::new("codex-cli")?.as_str(), "codex-cli");
    assert!(AgentBackendId::new("Codex CLI").is_err());
    assert!(AgentBackendId::new("").is_err());
    Ok::<(), String>(())
}
