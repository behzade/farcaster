use super::*;
fn counts(patches: &[&str]) -> Option<(usize, usize)> {
    let mut net = NetChanges::default();
    for patch in patches {
        net.apply(&unified_edits(patch)?)?;
    }
    net.counts()
}
#[test]
fn repeated_replacement_and_reversal() {
    let first = "@@ -1 +1 @@\n-old\n+middle";
    assert_eq!(counts(&[first, "@@ -1 +1 @@\n-middle\n+new"]), Some((1, 1)));
    assert_eq!(counts(&[first, "@@ -1 +1 @@\n-middle\n+old"]), Some((0, 0)));
}
#[test]
fn insert_then_edit_or_delete() {
    let first = "@@ -0,0 +1,2 @@\n+one\n+two";
    assert_eq!(counts(&[first, "@@ -2 +2 @@\n-two\n+three"]), Some((2, 0)));
    assert_eq!(
        counts(&[first, "@@ -1,2 +0,0 @@\n-one\n-two"]),
        Some((0, 0))
    );
}
#[test]
fn separated_hunks_and_shifted_lines() {
    assert_eq!(
        counts(&[
            "@@ -1 +1,2 @@\n-a\n+b\n+c\n@@ -8 +9 @@\n-x\n+y",
            "@@ -9 +9 @@\n-y\n+z"
        ]),
        Some((3, 2))
    );
}
#[test]
fn context_does_not_count_as_changes() {
    assert_eq!(
        counts(&["@@ -1,3 +1,3 @@\n same\n-old\n+new\n tail"]),
        Some((1, 1))
    );
}
#[test]
fn rejects_incomplete_conflicting_and_unpositioned_patches() {
    assert_eq!(counts(&["+new\n-old"]), None);
    assert_eq!(counts(&["@@ -1,2 +1 @@\n-old\n+new"]), None);
    assert_eq!(
        counts(&["@@ -1 +1 @@\n-old\n+new", "@@ -1 +1 @@\n-wrong\n+next"]),
        None
    );
}
