// Unified-patch parsing adapted from Pierre's parsePatchFiles.ts. This rewrite
// emits a compact semantic plan and rejects malformed hunks with located errors.

use super::{
    DiffPlanError, DiffPlanErrorKind, DiffPlanOptions, DiffPlanRow, DiffRenderPlan, FileChangeKind,
    FileDiffPlan,
    format::{
        HunkHeader, PatchLine, collect_lines, file_sections, parse_git_paths, parse_header_path,
        parse_hunk_header, too_many_lines, trim_line_end,
    },
    plan_builder::{FilePlanner, RawCell},
};

#[derive(Clone, Copy)]
enum LastSide {
    Context { retained: bool },
    Old,
    New,
}

pub(super) fn plan_patch(
    patch: &str,
    options: DiffPlanOptions,
) -> Result<DiffRenderPlan, DiffPlanError> {
    if patch.trim().is_empty() {
        return Err(DiffPlanError::new(
            DiffPlanErrorKind::EmptyPatch,
            1,
            "patch is empty",
        ));
    }
    let lines = collect_lines(patch);
    let sections = file_sections(&lines);
    if sections.is_empty() {
        return Err(DiffPlanError::new(
            DiffPlanErrorKind::MissingFileHeader,
            1,
            "patch has no file header",
        ));
    }

    let mut files = Vec::with_capacity(sections.len());
    for (start, end) in sections {
        files.push(plan_file(&lines[start..end], options)?);
    }
    Ok(DiffRenderPlan {
        files,
        source_bytes: patch.len(),
    })
}

fn plan_file(
    lines: &[PatchLine<'_>],
    options: DiffPlanOptions,
) -> Result<FileDiffPlan, DiffPlanError> {
    let first_line = lines.first().map_or(1, |line| line.number);
    let is_git = lines
        .first()
        .is_some_and(|line| line.raw.starts_with("diff --git "));
    let mut planner = FilePlanner::new(options);
    let mut index = 0;
    let mut saw_hunk = false;

    while index < lines.len() {
        let line = lines[index];
        let text = trim_line_end(line.raw);
        if text.starts_with("diff --git ") {
            if let Some((old, new)) = parse_git_paths(text) {
                planner.plan.old_path = Some(old);
                planner.plan.path = Some(new);
            }
            index += 1;
            continue;
        }
        if let Some(path) = text
            .strip_prefix("--- ")
            .and_then(|value| parse_header_path(value, is_git))
        {
            if path != "/dev/null" {
                planner.plan.old_path = Some(path);
            } else {
                planner.plan.old_path = None;
                planner.plan.kind = FileChangeKind::Added;
            }
            index += 1;
            continue;
        }
        if let Some(path) = text
            .strip_prefix("+++ ")
            .and_then(|value| parse_header_path(value, is_git))
        {
            if path != "/dev/null" {
                planner.plan.path = Some(path);
            } else {
                planner.plan.path = None;
                planner.plan.kind = FileChangeKind::Deleted;
            }
            index += 1;
            continue;
        }
        if let Some(path) = text.strip_prefix("rename from ") {
            planner.plan.old_path = Some(path.to_owned());
            planner.plan.kind = FileChangeKind::Renamed;
            index += 1;
            continue;
        }
        if let Some(path) = text.strip_prefix("rename to ") {
            planner.plan.path = Some(path.to_owned());
            planner.plan.kind = FileChangeKind::Renamed;
            index += 1;
            continue;
        }
        if text.starts_with("new file mode ") {
            planner.plan.kind = FileChangeKind::Added;
            index += 1;
            continue;
        }
        if text.starts_with("deleted file mode ") {
            planner.plan.kind = FileChangeKind::Deleted;
            index += 1;
            continue;
        }
        if text.starts_with("similarity index ") {
            planner.plan.kind = FileChangeKind::Renamed;
            index += 1;
            continue;
        }
        if text.starts_with("@@") {
            saw_hunk = true;
            let header = parse_hunk_header(text).ok_or_else(|| {
                DiffPlanError::new(
                    DiffPlanErrorKind::InvalidHunkHeader,
                    line.number,
                    "invalid hunk header",
                )
            })?;
            planner.push(DiffPlanRow::Hunk {
                header: text.to_owned(),
            });
            index = plan_hunk(lines, index + 1, header, &mut planner)?;
            continue;
        }
        index += 1;
    }

    if planner.plan.old_path.is_none() && planner.plan.path.is_none() {
        return Err(DiffPlanError::new(
            DiffPlanErrorKind::MissingFileHeader,
            first_line,
            "file has no path header",
        ));
    }
    if !is_git && !saw_hunk {
        return Err(DiffPlanError::new(
            DiffPlanErrorKind::MissingHunk,
            first_line,
            "unified file has no hunks",
        ));
    }
    if planner.plan.old_path != planner.plan.path
        && planner.plan.old_path.is_some()
        && planner.plan.path.is_some()
    {
        planner.plan.kind = FileChangeKind::Renamed;
    }
    Ok(planner.plan)
}

fn plan_hunk(
    lines: &[PatchLine<'_>],
    mut index: usize,
    header: HunkHeader,
    planner: &mut FilePlanner,
) -> Result<usize, DiffPlanError> {
    let mut old_number = header.old_start;
    let mut new_number = header.new_start;
    let mut consumed_old = 0;
    let mut consumed_new = 0;
    let mut pending_old = Vec::new();
    let mut pending_new = Vec::new();
    let mut last_side = None;

    while index < lines.len() {
        let line = lines[index];
        let counts_complete = consumed_old >= header.old_count && consumed_new >= header.new_count;
        if counts_complete && !line.raw.starts_with('\\') {
            break;
        }
        let mut chars = line.raw.chars();
        let prefix = chars.next().ok_or_else(|| {
            DiffPlanError::new(
                DiffPlanErrorKind::InvalidHunkLine,
                line.number,
                "empty hunk line",
            )
        })?;
        let content = chars.as_str();
        match prefix {
            ' ' => {
                if consumed_old >= header.old_count || consumed_new >= header.new_count {
                    return Err(too_many_lines(line.number));
                }
                planner.push_changes(
                    std::mem::take(&mut pending_old),
                    std::mem::take(&mut pending_new),
                );
                let retained =
                    planner.push_context(old_number, new_number, trim_line_end(content).to_owned());
                old_number = old_number.saturating_add(1);
                new_number = new_number.saturating_add(1);
                consumed_old += 1;
                consumed_new += 1;
                last_side = Some(LastSide::Context { retained });
            }
            '-' => {
                if consumed_old >= header.old_count {
                    return Err(too_many_lines(line.number));
                }
                pending_old.push(RawCell::new(old_number, trim_line_end(content).to_owned()));
                old_number = old_number.saturating_add(1);
                consumed_old += 1;
                planner.plan.deletions = planner.plan.deletions.saturating_add(1);
                last_side = Some(LastSide::Old);
            }
            '+' => {
                if consumed_new >= header.new_count {
                    return Err(too_many_lines(line.number));
                }
                pending_new.push(RawCell::new(new_number, trim_line_end(content).to_owned()));
                new_number = new_number.saturating_add(1);
                consumed_new += 1;
                planner.plan.additions = planner.plan.additions.saturating_add(1);
                last_side = Some(LastSide::New);
            }
            '\\' => mark_no_newline(last_side, &mut pending_old, &mut pending_new, planner),
            _ => {
                return Err(DiffPlanError::new(
                    DiffPlanErrorKind::InvalidHunkLine,
                    line.number,
                    "invalid hunk line prefix",
                ));
            }
        }
        index += 1;
    }
    planner.push_changes(pending_old, pending_new);
    if consumed_old != header.old_count || consumed_new != header.new_count {
        let line = lines
            .get(index.saturating_sub(1))
            .map_or(1, |line| line.number);
        return Err(DiffPlanError::new(
            DiffPlanErrorKind::HunkLineCountMismatch,
            line,
            "hunk line count does not match its header",
        ));
    }
    if let Some(line) = lines.get(index)
        && is_hunk_body_line(line.raw)
        && !is_format_patch_separator(line.raw)
    {
        return Err(too_many_lines(line.number));
    }
    Ok(index)
}

fn is_hunk_body_line(line: &str) -> bool {
    matches!(line.as_bytes().first(), Some(b' ' | b'-' | b'+'))
}

fn is_format_patch_separator(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("--") else {
        return false;
    };
    rest.chars().all(char::is_whitespace)
}

fn mark_no_newline(
    side: Option<LastSide>,
    pending_old: &mut [RawCell],
    pending_new: &mut [RawCell],
    planner: &mut FilePlanner,
) {
    match side {
        Some(LastSide::Old) => {
            if let Some(cell) = pending_old.last_mut() {
                cell.no_newline = true;
            }
        }
        Some(LastSide::New) => {
            if let Some(cell) = pending_new.last_mut() {
                cell.no_newline = true;
            }
        }
        Some(LastSide::Context { retained: true }) => planner.mark_retained_context_no_newline(),
        Some(LastSide::Context { retained: false }) => {}
        None => {}
    }
}
