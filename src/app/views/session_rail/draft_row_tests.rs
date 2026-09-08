use super::draft_can_be_discarded;

#[test]
fn failed_submitted_drafts_can_be_discarded() {
    assert!(draft_can_be_discarded(true, "Failed"));
    assert!(!draft_can_be_discarded(true, "Working"));
    assert!(draft_can_be_discarded(false, "Draft"));
}
