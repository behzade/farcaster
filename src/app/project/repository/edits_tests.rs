use super::*;

#[test]
fn checkboxes_select_directly_and_deselect_the_last_path() {
    let mut selection = FileSelection::default();
    selection.toggle("a".into());
    assert_eq!(selection.paths, BTreeSet::from([PathBuf::from("a")]));
    selection.toggle("b".into());
    selection.toggle("a".into());
    assert_eq!(selection.paths, BTreeSet::from([PathBuf::from("b")]));
    selection.toggle("b".into());
    assert!(selection.paths.is_empty());
}
