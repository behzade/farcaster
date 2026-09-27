use super::*;

#[test]
fn navigation_bounds_and_activation_follow_filtered_results() {
    assert_eq!(navigation(0, 0, "enter"), None);
    assert_eq!(navigation(0, 3, "up"), Some((0, false)));
    assert_eq!(navigation(0, 3, "down"), Some((1, false)));
    assert_eq!(navigation(2, 3, "down"), Some((2, false)));
    assert_eq!(navigation(8, 2, "enter"), Some((1, true)));
    assert_eq!(navigation(8, 2, "up"), Some((0, false)));
    assert_eq!(navigation(1, 3, "escape"), None);
}

#[test]
fn matching_keeps_id_name_and_combined_substrings() {
    assert!(model_matches("Model-ID", "Display Name", "model-id"));
    assert!(model_matches("Model-ID", "Display Name", "display"));
    assert!(model_matches("Model-ID", "Display Name", "id display"));
    assert!(model_matches("Model-ID", "Display Name", ""));
    assert!(!model_matches("Model-ID", "Display Name", "other"));
}
