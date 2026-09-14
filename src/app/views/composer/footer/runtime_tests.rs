use super::*;

#[test]
fn sandbox_labels_and_colors_distinguish_unrestricted_access() {
    use HarnessAccessMode::{Auto, Full, Sandboxed};
    assert_eq!(access_mode_label(Sandboxed), "Sandbox: On");
    assert_eq!(access_mode_label(Full), "Sandbox: Off");
    assert_eq!(access_mode_label(Auto), "Sandbox: Auto");
    assert_eq!(access_mode_color(Sandboxed), THEME.colors.muted);
    assert_eq!(access_mode_color(Full), THEME.colors.warning);
}

#[test]
fn sandbox_transition_keeps_requested_label_and_explains_effective_mode() {
    use HarnessAccessMode::{Full, Sandboxed};
    assert_eq!(
        sandbox_state_label(SandboxState::Checking, Sandboxed),
        "Sandbox: On"
    );
    assert_eq!(
        sandbox_state_label(SandboxState::Failed, Sandboxed),
        "Sandbox: Unavailable"
    );
    let state = SandboxState::Pending(Full);
    assert_eq!(sandbox_state_label(state, Sandboxed), "Sandbox: On");
    let tooltip = sandbox_state_tooltip(state, Sandboxed);
    assert!(tooltip.contains("Currently Sandbox: Off"));
    assert!(tooltip.contains("apply Sandbox: On"));
}
