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
fn sandbox_labels_do_not_claim_protection_without_confirmation() {
    use crate::agents::SandboxState;
    assert_eq!(
        sandbox_state_label(SandboxState::Checking),
        "Sandbox: Checking"
    );
    assert_eq!(
        sandbox_state_label(SandboxState::Failed),
        "Sandbox: Unavailable"
    );
    assert_eq!(
        sandbox_state_label(SandboxState::Active(HarnessAccessMode::Sandboxed)),
        "Sandbox: On"
    );
}
