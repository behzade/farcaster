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
