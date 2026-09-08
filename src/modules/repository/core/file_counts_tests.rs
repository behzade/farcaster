use super::*;

#[test]
fn file_totals_handle_nested_deleted_renamed_and_quoted_paths() {
    let patch = concat!(
        "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n---old\n+++new\n",
        "diff --git a/src/nested/deleted.rs b/src/nested/deleted.rs\n--- a/src/nested/deleted.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
        "diff --git a/old.rs b/new.rs\nsimilarity index 100%\nrename from old.rs\nrename to new.rs\n",
        "diff --git a/quoted b/quoted\n--- /dev/null\n+++ \"b/src/\\303\\251\\tfile.rs\"\n@@ -0,0 +1 @@\n+hello\n",
    );
    let counts = parse(patch);
    for (path, expected) in [
        ("src/main.rs", (1, 1)),
        ("src/nested/deleted.rs", (0, 1)),
        ("new.rs", (0, 0)),
        ("src/é\tfile.rs", (1, 0)),
    ] {
        assert_eq!(counts.get(Path::new(path)), Some(&Some(expected)), "{path}");
    }
}
