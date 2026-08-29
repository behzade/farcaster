use super::*;

fn options(layout: DiffLayout) -> DiffPlanOptions {
    DiffPlanOptions::new(layout)
}

#[test]
fn split_pairs_replacements_and_preserves_line_numbers() {
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -9,3 +9,3 @@\n same\n-old\n+new\n tail\n";
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");
    let rows = &plan.files[0].rows;
    assert_eq!(rows.len(), 4);
    let DiffPlanRow::Line(change) = &rows[2] else {
        panic!("expected change row");
    };
    assert_eq!(change.old.as_ref().map(|cell| cell.line_number), Some(10));
    assert_eq!(change.new.as_ref().map(|cell| cell.line_number), Some(10));
}

#[test]
fn unified_keeps_all_deletions_before_all_additions_in_a_change_block() {
    let patch = "--- a.txt\n+++ a.txt\n@@ -1,2 +1,2 @@\n-old one\n-old two\n+new one\n+new two\n";
    let plan = plan_patch(patch, options(DiffLayout::Unified)).expect("valid patch");
    let lines = plan.files[0]
        .rows
        .iter()
        .skip(1)
        .map(|row| match row {
            DiffPlanRow::Hunk { .. } => "hunk",
            DiffPlanRow::Line(line) if line.old.is_some() => "old",
            DiffPlanRow::Line(_) => "new",
        })
        .collect::<Vec<_>>();

    assert_eq!(lines, ["old", "old", "new", "new"]);
}

#[test]
fn similarity_alignment_keeps_extra_blanks_above_edited_lines() {
    let added_blank = "--- a.ts\n+++ a.ts\n@@ -1 +1,2 @@\n-oldCall()\n+\n+oldCalls()\n";
    let plan = plan_patch(added_blank, options(DiffLayout::Split)).expect("valid patch");
    let [
        DiffPlanRow::Hunk { .. },
        DiffPlanRow::Line(blank),
        DiffPlanRow::Line(pair),
    ] = plan.files[0].rows.as_slice()
    else {
        panic!("expected blank then aligned pair");
    };
    assert!(blank.old.is_none());
    assert_eq!(blank.new.as_ref().map(|cell| cell.text.as_str()), Some(""));
    assert_eq!(
        pair.old.as_ref().map(|cell| cell.text.as_str()),
        Some("oldCall()")
    );
    assert_eq!(
        pair.new.as_ref().map(|cell| cell.text.as_str()),
        Some("oldCalls()")
    );

    let removed_blank = "--- a.ts\n+++ a.ts\n@@ -1,2 +1 @@\n-\n-oldCalls()\n+oldCall()\n";
    let plan = plan_patch(removed_blank, options(DiffLayout::Split)).expect("valid patch");
    let [
        DiffPlanRow::Hunk { .. },
        DiffPlanRow::Line(blank),
        DiffPlanRow::Line(pair),
    ] = plan.files[0].rows.as_slice()
    else {
        panic!("expected blank then aligned pair");
    };
    assert!(blank.new.is_none());
    assert_eq!(blank.old.as_ref().map(|cell| cell.text.as_str()), Some(""));
    assert_eq!(
        pair.old.as_ref().map(|cell| cell.text.as_str()),
        Some("oldCalls()")
    );
    assert_eq!(
        pair.new.as_ref().map(|cell| cell.text.as_str()),
        Some("oldCall()")
    );
}

#[test]
fn hunk_body_markers_that_resemble_headers_are_content() {
    let patch = "--- markers.txt\n+++ markers.txt\n@@ -1 +1 @@\n--- old marker\n+++ new marker\n";
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");
    let DiffPlanRow::Line(line) = &plan.files[0].rows[1] else {
        panic!("expected line");
    };
    assert_eq!(
        line.old.as_ref().map(|cell| cell.text.as_str()),
        Some("-- old marker")
    );
    assert_eq!(
        line.new.as_ref().map(|cell| cell.text.as_str()),
        Some("++ new marker")
    );
}

#[test]
fn bounded_plan_keeps_complete_statistics() {
    let patch = "--- a.txt\n+++ a.txt\n@@ -1,3 +1,3 @@\n-a\n-b\n-c\n+x\n+y\n+z\n";
    let plan =
        plan_patch(patch, DiffPlanOptions::bounded(DiffLayout::Unified, 2)).expect("valid patch");
    let file = &plan.files[0];
    assert_eq!(file.rows.len(), 2);
    assert_eq!(file.total_rows(), 7);
    assert_eq!((file.additions, file.deletions), (3, 3));
}

#[test]
fn plain_unified_paths_keep_leading_a_and_b_components() {
    let patch = "--- a/source.txt\n+++ b/source.txt\n@@ -1 +1 @@\n-old\n+new\n";
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");

    assert_eq!(plan.files[0].old_path.as_deref(), Some("a/source.txt"));
    assert_eq!(plan.files[0].path.as_deref(), Some("b/source.txt"));
}

#[test]
fn malformed_git_hunk_headers_are_rejected() {
    let patch =
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ malformed @@\n-old\n+new\n";
    let error = plan_patch(patch, options(DiffLayout::Split)).expect_err("malformed hunk");

    assert_eq!(error.kind, DiffPlanErrorKind::InvalidHunkHeader);
}

#[test]
fn line_count_mismatches_are_rejected() {
    let patch = "--- a.txt\n+++ a.txt\n@@ -1,2 +1 @@\n-old\n+new\n";
    let error = plan_patch(patch, options(DiffLayout::Unified)).expect_err("invalid patch");
    assert_eq!(error.kind, DiffPlanErrorKind::HunkLineCountMismatch);
}

#[test]
fn fake_unified_headers_after_a_hunk_cannot_create_a_hunkless_file() {
    let patch = concat!(
        "--- markers.txt\n+++ markers.txt\n",
        "@@ -1 +1 @@\n",
        "--- old marker\n+++ new marker\n",
        "--- fake-old-marker\n+++ fake-new-marker\n",
    );

    let error = plan_patch(patch, options(DiffLayout::Unified)).expect_err("fake file");
    assert_eq!(error.kind, DiffPlanErrorKind::MissingHunk);
}

#[test]
fn extra_hunk_body_lines_are_rejected_but_format_patch_separators_are_allowed() {
    let extra = "--- a.txt\n+++ a.txt\n@@ -1 +1 @@\n-old\n+new\n-extra\n";
    let trailer = "--- a.txt\n+++ a.txt\n@@ -1 +1,2 @@\n old\n+new\n-- \n2.52.0\n";

    let error = plan_patch(extra, options(DiffLayout::Unified)).expect_err("extra line");
    assert_eq!(error.kind, DiffPlanErrorKind::HunkLineCountMismatch);
    plan_patch(trailer, options(DiffLayout::Unified)).expect("format-patch trailer");
}

#[test]
fn multi_file_git_patches_keep_independent_paths_and_statistics() {
    let patch = concat!(
        "diff --git a/old.rs b/new.rs\n",
        "similarity index 90%\n",
        "rename from old.rs\n",
        "rename to new.rs\n",
        "--- a/old.rs\n+++ b/new.rs\n",
        "@@ -1 +1 @@\n-old\n+new\n",
        "diff --git a/created.txt b/created.txt\n",
        "new file mode 100644\n",
        "--- /dev/null\n+++ b/created.txt\n",
        "@@ -0,0 +1 @@\n+created\n",
    );
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");

    assert_eq!(plan.files.len(), 2);
    assert_eq!(plan.files[0].kind, FileChangeKind::Renamed);
    assert_eq!(plan.files[0].old_path.as_deref(), Some("old.rs"));
    assert_eq!(plan.files[0].path.as_deref(), Some("new.rs"));
    assert_eq!(plan.files[1].kind, FileChangeKind::Added);
    assert_eq!((plan.files[1].additions, plan.files[1].deletions), (1, 0));
}

#[test]
fn deleted_files_keep_only_the_old_path() {
    let patch = concat!(
        "diff --git a/gone.txt b/gone.txt\n",
        "deleted file mode 100644\n",
        "--- a/gone.txt\n+++ /dev/null\n",
        "@@ -1 +0,0 @@\n-gone\n",
    );
    let plan = plan_patch(patch, options(DiffLayout::Unified)).expect("valid patch");
    let file = &plan.files[0];

    assert_eq!(file.kind, FileChangeKind::Deleted);
    assert_eq!(file.old_path.as_deref(), Some("gone.txt"));
    assert_eq!(file.path, None);
}

#[test]
fn no_newline_markers_attach_to_the_preceding_side() {
    let patch = concat!(
        "--- a.txt\n+++ a.txt\n",
        "@@ -1 +1 @@\n",
        "-old\n",
        "\\ No newline at end of file\n",
        "+new\n",
        "\\ No newline at end of file\n",
    );
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");
    let DiffPlanRow::Line(line) = &plan.files[0].rows[1] else {
        panic!("expected line");
    };

    assert_eq!(line.old.as_ref().map(|cell| cell.no_newline), Some(true));
    assert_eq!(line.new.as_ref().map(|cell| cell.no_newline), Some(true));
}

#[test]
fn omitted_no_newline_context_does_not_mark_the_last_retained_row() {
    let patch = concat!(
        "--- a.txt\n+++ a.txt\n",
        "@@ -1,2 +1,2 @@\n",
        " first\n",
        " second\n",
        "\\ No newline at end of file\n",
    );
    let plan =
        plan_patch(patch, DiffPlanOptions::bounded(DiffLayout::Split, 2)).expect("valid patch");
    let DiffPlanRow::Line(first_context) = &plan.files[0].rows[1] else {
        panic!("expected retained context");
    };

    assert_eq!(
        first_context.old.as_ref().map(|cell| cell.no_newline),
        Some(false)
    );
    assert_eq!(
        first_context.new.as_ref().map(|cell| cell.no_newline),
        Some(false)
    );
}

#[test]
fn quoted_git_paths_with_spaces_are_preserved() {
    let patch = concat!(
        "diff --git \"a/old name.txt\" \"b/new name.txt\"\n",
        "similarity index 100%\n",
        "rename from old name.txt\n",
        "rename to new name.txt\n",
    );
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");

    assert_eq!(plan.files[0].old_path.as_deref(), Some("old name.txt"));
    assert_eq!(plan.files[0].path.as_deref(), Some("new name.txt"));
}

#[test]
fn git_c_quoted_paths_are_decoded() {
    let patch = concat!(
        "diff --git \"a/caf\\303\\251.txt\" \"b/tab\\tname.txt\"\n",
        "similarity index 100%\n",
        "rename from \"caf\\303\\251.txt\"\n",
        "rename to \"tab\\tname.txt\"\n",
    );
    let plan = plan_patch(patch, options(DiffLayout::Split)).expect("valid patch");

    assert_eq!(plan.files[0].old_path.as_deref(), Some("café.txt"));
    assert_eq!(plan.files[0].path.as_deref(), Some("tab\tname.txt"));
}

#[test]
fn empty_and_headerless_inputs_are_distinct_errors() {
    let empty = plan_patch("", options(DiffLayout::Split)).expect_err("empty patch");
    let headerless = plan_patch("@@ -1 +1 @@\n-old\n+new\n", options(DiffLayout::Split))
        .expect_err("headerless patch");

    assert_eq!(empty.kind, DiffPlanErrorKind::EmptyPatch);
    assert_eq!(headerless.kind, DiffPlanErrorKind::MissingFileHeader);
}
