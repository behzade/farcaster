use super::*;

#[test]
fn native_process_uses_selected_harness_permissions() {
    let arguments = |mode| {
        let mut command = std::process::Command::new("codex");
        configure_permissions(&mut command, mode);
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    };
    use crate::agents::HarnessAccessMode::{Auto, Full, Sandboxed};
    assert_eq!(
        arguments(Full),
        ["--dangerously-bypass-approvals-and-sandbox"]
    );
    assert_eq!(
        arguments(Sandboxed),
        [
            "--sandbox",
            "workspace-write",
            "--ask-for-approval",
            "on-request",
            "-c",
            "approvals_reviewer=\"user\""
        ]
    );
    assert_eq!(arguments(Auto), ["--approve-for-me"]);
    assert_eq!(approvals_reviewer(Auto), "auto_review");
    assert_eq!(approvals_reviewer(Sandboxed), "user");
    assert_eq!(approvals_reviewer(Full), "user");
}

#[test]
fn descriptor_keeps_codex_specific_features_independent() {
    let capabilities = descriptor().capabilities;
    assert_eq!(capabilities.turns.queue, CapabilitySupport::Available);
    assert_eq!(capabilities.turns.follow_up, CapabilitySupport::Available);
    assert_eq!(
        capabilities.observation.child_agents,
        CapabilitySupport::Available
    );
}
