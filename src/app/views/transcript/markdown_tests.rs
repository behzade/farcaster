use super::RecentCache;

#[test]
fn recently_used_markdown_state_survives_virtualization() {
    let mut cache = RecentCache::new(2);
    let mut parses = 0;
    let (first, first_hit) = cache.get_or_insert_with("final-row", || {
        parses += 1;
        "parsed state"
    });
    let _other = cache.get_or_insert_with("other-row", || "other state");
    let (restored, restored_hit) = cache.get_or_insert_with("final-row", || {
        parses += 1;
        "replacement state"
    });

    assert_eq!(first, restored);
    assert!(!first_hit);
    assert!(restored_hit);
    assert_eq!(parses, 1);
}
