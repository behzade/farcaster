use super::*;

#[test]
fn leaving_selection_mode_clears_paths_and_inactive_rows_cannot_select() {
    let mut selection = FileSelection::default();
    selection.toggle("a".into());
    assert!(selection.paths.is_empty());
    selection.toggle_mode();
    selection.toggle("a".into());
    selection.toggle("b".into());
    selection.toggle("a".into());
    assert_eq!(selection.paths, BTreeSet::from([PathBuf::from("b")]));
    selection.toggle_mode();
    assert!(!selection.active);
    assert!(selection.paths.is_empty());
}
