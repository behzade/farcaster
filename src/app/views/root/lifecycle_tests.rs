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

#[test]
fn overlays_cover_the_native_surface_hold_it_hidden_and_release_it_when_they_expire() {
    assert_eq!(
        native_surface_action(false, true, true),
        NativeSurfaceAction::Cover
    );
    assert_eq!(
        native_surface_action(true, false, true),
        NativeSurfaceAction::Restore
    );
    assert_eq!(
        native_surface_action(true, true, true),
        NativeSurfaceAction::Hold
    );
    assert_eq!(
        native_surface_action(true, true, false),
        NativeSurfaceAction::Hold
    );
    assert_eq!(
        native_surface_action(false, false, true),
        NativeSurfaceAction::None
    );
    assert_eq!(
        native_surface_action(false, true, false),
        NativeSurfaceAction::None
    );
}
