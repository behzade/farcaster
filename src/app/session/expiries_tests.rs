use super::*;

#[test]
fn completion_expiry_removes_only_the_matching_completion() {
    let completed_at = Instant::now();
    let replacement = completed_at + Duration::from_secs(1);
    let mut completions = HashMap::from([("session:a".into(), replacement)]);
    let mut statuses = HashMap::from([("session:a".into(), "Done".into())]);

    assert!(!expire_recent_completion(
        &mut completions,
        &mut statuses,
        "session:a",
        completed_at,
    ));
    assert_eq!(statuses.get("session:a").map(String::as_str), Some("Done"));

    assert!(expire_recent_completion(
        &mut completions,
        &mut statuses,
        "session:a",
        replacement,
    ));
    assert!(!completions.contains_key("session:a"));
    assert!(!statuses.contains_key("session:a"));
}
