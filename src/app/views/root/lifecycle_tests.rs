use super::*;

#[test]
fn final_external_dismissal_runs_focus_restore_instead_of_dialog_setup() {
    assert_eq!(
        dialog_lifecycle_action(true, false),
        DialogLifecycleAction::RestoreFocus
    );
    assert_eq!(
        dialog_lifecycle_action(true, true),
        DialogLifecycleAction::Setup
    );
    assert_eq!(
        dialog_lifecycle_action(false, false),
        DialogLifecycleAction::None
    );
}
