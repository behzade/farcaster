use super::*;
#[test]
fn keeps_indentation_and_positions_after_insertions() {
    assert_eq!(
        unified_diff(" 1 context\n-2 old\n+2   new\n+3 extra\n 3 tail\n   ...\n-9 later\n+10 last"),
        Some("@@ -2,1 +2,2 @@\n-old\n+  new\n+extra\n@@ -9,1 +10,1 @@\n-later\n+last\n".into())
    );
}
#[test]
fn insertions_and_deletions_use_empty_range_positions() {
    assert_eq!(
        unified_diff("+1 new"),
        Some("@@ -0,0 +1,1 @@\n+new\n".into())
    );
    assert_eq!(
        unified_diff("-1 old"),
        Some("@@ -1,1 +0,0 @@\n-old\n".into())
    );
    assert_eq!(unified_diff("-1 old\n+3 wrong"), None);
}
