use pi_gpui::diff_plan::{DiffLayout, DiffPlanOptions, DiffPlanRow, DiffRenderPlan, plan_patch};

const SMALL_PATCH: &str = include_str!("fixtures/diff_plan/small-pi-ai.patch");
const MEDIUM_PATCH: &str = include_str!("fixtures/diff_plan/medium-pi-web.patch");
const STRESS_PATCH: &str = include_str!("fixtures/diff_plan/stress-pierre.patch");
const MIXED_PATCH: &str = include_str!("fixtures/diff_plan/mixed-pi-coding-agent.patch");

#[test]
fn real_corpora_match_independent_counts_in_both_layouts() {
    let cases = [
        (SMALL_PATCH, (7, 9, 0)),
        (MEDIUM_PATCH, (4, 52, 17)),
        (MIXED_PATCH, (2, 56, 44)),
        (STRESS_PATCH, (99, 1_548, 1_516)),
    ];

    for (patch, expected) in cases {
        let scan = scan_patch(patch);
        assert_eq!((scan.files, scan.additions, scan.deletions), expected);

        let unified = plan(patch, DiffLayout::Unified);
        let split = plan(patch, DiffLayout::Split);
        for plan in [&unified, &split] {
            let hunk_rows = plan
                .files
                .iter()
                .flat_map(|file| &file.rows)
                .filter(|row| matches!(row, DiffPlanRow::Hunk { .. }))
                .count();
            assert_eq!(plan.files.len(), scan.files);
            assert_eq!(hunk_rows, scan.hunks);
            assert_eq!(change_counts(plan), (scan.additions, scan.deletions));
            assert_plan_invariants(plan);
        }
        assert!(split.total_rows() <= unified.total_rows());
    }
}

#[test]
fn stress_plan_handles_pierres_real_multi_commit_patch() {
    let plan = plan(STRESS_PATCH, DiffLayout::Split);

    assert_eq!(
        plan.files
            .iter()
            .filter(|file| file.kind == pi_gpui::diff_plan::FileChangeKind::Renamed)
            .count(),
        13
    );
    assert!(plan.files.iter().any(|file| {
        file.kind == pi_gpui::diff_plan::FileChangeKind::Renamed && file.rows.is_empty()
    }));
    assert!(plan.files.iter().any(|file| {
        file.kind == pi_gpui::diff_plan::FileChangeKind::Deleted
            && file.path.is_none()
            && file.old_path.is_some()
    }));
    assert_eq!(no_newline_cells(&plan), 2);
    assert_plan_invariants(&plan);
}

#[test]
fn bounded_rows_are_exact_prefixes_of_each_real_file_plan() {
    for layout in [DiffLayout::Unified, DiffLayout::Split] {
        let full = plan(MIXED_PATCH, layout);
        for maximum in [0, 1, 12] {
            let bounded = plan_patch(MIXED_PATCH, DiffPlanOptions::bounded(layout, maximum))
                .expect("real patch should parse");
            assert_eq!(change_counts(&bounded), change_counts(&full));
            assert_plan_invariants(&bounded);
            for (full_file, bounded_file) in full.files.iter().zip(&bounded.files) {
                assert_eq!(
                    bounded_file.rows,
                    full_file.rows[..maximum.min(full_file.rows.len())]
                );
                assert_eq!(bounded_file.total_rows(), full_file.total_rows());
            }
        }
    }
}

#[test]
fn crlf_and_lf_real_patches_have_the_same_semantic_plan() {
    let lf = plan(MEDIUM_PATCH, DiffLayout::Split);
    let crlf_patch = MEDIUM_PATCH.replace('\n', "\r\n");
    let crlf = plan(&crlf_patch, DiffLayout::Split);

    assert_eq!(crlf.files, lf.files);
}

#[test]
fn disabling_intraline_changes_changes_only_the_span_ranges() {
    let with_spans = plan(MEDIUM_PATCH, DiffLayout::Split);
    let mut options = DiffPlanOptions::new(DiffLayout::Split);
    options.intraline_changes = false;
    let without_spans = plan_patch(MEDIUM_PATCH, options).expect("real patch should parse");

    assert_eq!(without_spans, without_changed_ranges(with_spans));
}

#[derive(Default)]
struct PatchScan {
    files: usize,
    hunks: usize,
    additions: usize,
    deletions: usize,
}

fn scan_patch(patch: &str) -> PatchScan {
    let mut scan = PatchScan::default();
    let mut remaining_old = 0;
    let mut remaining_new = 0;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            scan.files += 1;
        }
        if remaining_old == 0 && remaining_new == 0 {
            if let Some((old, new)) = scan_hunk_counts(line) {
                scan.hunks += 1;
                remaining_old = old;
                remaining_new = new;
            }
            continue;
        }
        match line.as_bytes().first().copied() {
            Some(b' ') => {
                remaining_old = remaining_old.saturating_sub(1);
                remaining_new = remaining_new.saturating_sub(1);
            }
            Some(b'-') => {
                remaining_old = remaining_old.saturating_sub(1);
                scan.deletions += 1;
            }
            Some(b'+') => {
                remaining_new = remaining_new.saturating_sub(1);
                scan.additions += 1;
            }
            _ => {}
        }
    }
    scan
}

fn scan_hunk_counts(line: &str) -> Option<(usize, usize)> {
    let mut fields = line.split_whitespace();
    (fields.next()? == "@@").then_some(())?;
    let old = fields.next()?.strip_prefix('-')?;
    let new = fields.next()?.strip_prefix('+')?;
    Some((scan_range_count(old)?, scan_range_count(new)?))
}

fn scan_range_count(range: &str) -> Option<usize> {
    range
        .split_once(',')
        .map_or(Some(1), |(_, count)| count.parse().ok())
}

fn plan(patch: &str, layout: DiffLayout) -> DiffRenderPlan {
    plan_patch(patch, DiffPlanOptions::new(layout)).expect("real patch should parse")
}

fn without_changed_ranges(mut plan: DiffRenderPlan) -> DiffRenderPlan {
    for file in &mut plan.files {
        for row in &mut file.rows {
            let DiffPlanRow::Line(line) = row else {
                continue;
            };
            if let Some(old) = line.old.as_mut() {
                old.changed.clear();
            }
            if let Some(new) = line.new.as_mut() {
                new.changed.clear();
            }
        }
    }
    plan
}

fn no_newline_cells(plan: &DiffRenderPlan) -> usize {
    plan.files
        .iter()
        .flat_map(|file| &file.rows)
        .filter_map(|row| match row {
            DiffPlanRow::Hunk { .. } => None,
            DiffPlanRow::Line(line) => Some(line),
        })
        .flat_map(|line| [line.old.as_ref(), line.new.as_ref()])
        .flatten()
        .filter(|cell| cell.no_newline)
        .count()
}

fn change_counts(plan: &DiffRenderPlan) -> (usize, usize) {
    plan.files.iter().fold((0, 0), |counts, file| {
        (counts.0 + file.additions, counts.1 + file.deletions)
    })
}

fn assert_plan_invariants(plan: &DiffRenderPlan) {
    for file in &plan.files {
        assert!(file.path.is_some() || file.old_path.is_some());
        for row in &file.rows {
            let DiffPlanRow::Line(line) = row else {
                continue;
            };
            for cell in [line.old.as_ref(), line.new.as_ref()].into_iter().flatten() {
                assert!(cell.line_number > 0);
                for range in &cell.changed {
                    assert!(range.start < range.end);
                    assert!(range.end <= cell.text.len());
                    assert!(cell.text.is_char_boundary(range.start));
                    assert!(cell.text.is_char_boundary(range.end));
                }
                assert!(
                    cell.changed
                        .windows(2)
                        .all(|ranges| ranges[0].end <= ranges[1].start)
                );
            }
            if let (Some(old), Some(new)) = (&line.old, &line.new)
                && old.text == new.text
            {
                assert!(old.changed.is_empty());
                assert!(new.changed.is_empty());
            }
        }
    }
}
