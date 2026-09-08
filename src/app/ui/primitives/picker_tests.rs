use super::*;

#[test]
fn search_matches_labels_details_and_keywords_by_term() {
    let row = PickerRow::new(
        "session",
        AppIcon::MagnifyingGlass,
        "Find session",
        Some("/work/pi".into()),
        None,
        "resume thread",
    );

    assert!(row.matches("find pi"));
    assert!(row.matches("resume"));
    assert!(!row.matches("project settings"));
}
