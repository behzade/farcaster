use super::*;

#[test]
fn mention_query_uses_the_token_at_the_cursor() {
    assert_eq!(
        query_at_cursor("read @src/ma please", 12),
        Some(MentionQuery {
            range: 5..12,
            text: "src/ma".into(),
        })
    );
    assert!(query_at_cursor("email@example", 13).is_none());
    assert!(query_at_cursor("@two words", 10).is_none());
}

#[test]
fn matching_is_fuzzy_and_prefers_file_names() {
    let files = vec![
        "src/main.rs".into(),
        "docs/runtime.md".into(),
        "main.txt".into(),
    ];
    assert_eq!(matches(&files, "main"), ["main.txt", "src/main.rs"]);
    assert_eq!(matches(&files, "srm"), ["src/main.rs", "docs/runtime.md"]);
}

#[test]
fn empty_matching_keeps_every_file_in_stable_order() {
    let files = vec!["z.rs".into(), "a.rs".into()];
    assert_eq!(matches(&files, ""), ["a.rs", "z.rs"]);
}

#[test]
fn unicode_matching_uses_character_boundaries() {
    let files = vec!["src/café.rs".into(), "src/cafeteria.rs".into()];
    assert_eq!(matches(&files, "café"), ["src/café.rs"]);
}

#[test]
fn insertion_replaces_only_the_active_token_and_tracks_byte_cursor() {
    let query = query_at_cursor("🙂 see @ma now", 12).expect("query");
    let (text, cursor) = insert("🙂 see @ma now", &query, "src/main.rs");
    assert_eq!(text, "🙂 see @src/main.rs  now");
    assert_eq!(cursor, 22);
}
