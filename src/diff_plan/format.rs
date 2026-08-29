// Patch framing and header parsing adapted from Pierre's parsePatchFiles.ts.

use super::{DiffPlanError, DiffPlanErrorKind};

#[derive(Clone, Copy)]
pub(super) struct PatchLine<'a> {
    pub number: usize,
    pub raw: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct HunkHeader {
    pub old_start: u64,
    pub old_count: usize,
    pub new_start: u64,
    pub new_count: usize,
}

pub(super) fn collect_lines(patch: &str) -> Vec<PatchLine<'_>> {
    patch
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, raw)| PatchLine {
            number: index + 1,
            raw,
        })
        .collect()
}

pub(super) fn file_sections(lines: &[PatchLine<'_>]) -> Vec<(usize, usize)> {
    let git_starts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.raw.starts_with("diff --git ").then_some(index))
        .collect::<Vec<_>>();
    if !git_starts.is_empty() {
        return ranges_from_starts(&git_starts, lines.len());
    }

    // A hunk body may itself contain `---` and `+++`. Track its declared side
    // counts so only true adjacent file headers create section boundaries.
    let mut starts = Vec::new();
    let mut index = 0;
    let mut remaining_old = 0;
    let mut remaining_new = 0;
    while index + 1 < lines.len() {
        if remaining_old == 0
            && remaining_new == 0
            && lines[index].raw.starts_with("--- ")
            && lines[index + 1].raw.starts_with("+++ ")
        {
            starts.push(index);
            index += 2;
            continue;
        }
        if remaining_old == 0
            && remaining_new == 0
            && let Some(header) = parse_hunk_header(trim_line_end(lines[index].raw))
        {
            remaining_old = header.old_count;
            remaining_new = header.new_count;
        } else if remaining_old > 0 || remaining_new > 0 {
            match lines[index].raw.as_bytes().first().copied() {
                Some(b' ') => {
                    remaining_old = remaining_old.saturating_sub(1);
                    remaining_new = remaining_new.saturating_sub(1);
                }
                Some(b'-') => remaining_old = remaining_old.saturating_sub(1),
                Some(b'+') => remaining_new = remaining_new.saturating_sub(1),
                _ => {}
            }
        }
        index += 1;
    }
    ranges_from_starts(&starts, lines.len())
}

pub(super) fn parse_git_paths(line: &str) -> Option<(String, String)> {
    let (old, new) = split_shellish_pair(line.strip_prefix("diff --git ")?)?;
    Some((strip_git_prefix(&old), strip_git_prefix(&new)))
}

pub(super) fn parse_header_path(value: &str, is_git: bool) -> Option<String> {
    let path = value.split(['\t', '\r', '\n']).next()?.trim();
    if path.is_empty() {
        return None;
    }
    if !is_git {
        return Some(path.to_owned());
    }
    let path = parse_git_metadata_path(path)?;
    if path == "/dev/null" {
        Some(path)
    } else {
        Some(strip_git_prefix(&path))
    }
}

pub(super) fn parse_git_metadata_path(value: &str) -> Option<String> {
    let (path, rest) = parse_git_field(value)?;
    rest.trim().is_empty().then_some(path)
}

pub(super) fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Some(HunkHeader {
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

pub(super) fn trim_line_end(value: &str) -> &str {
    value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value)
}

pub(super) fn too_many_lines(line: usize) -> DiffPlanError {
    DiffPlanError::new(
        DiffPlanErrorKind::HunkLineCountMismatch,
        line,
        "hunk has more lines than its header declares",
    )
}

fn ranges_from_starts(starts: &[usize], end: usize) -> Vec<(usize, usize)> {
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| (*start, starts.get(index + 1).copied().unwrap_or(end)))
        .collect()
}

fn split_shellish_pair(value: &str) -> Option<(String, String)> {
    let (old, rest) = parse_git_field(value)?;
    let (new, rest) = parse_git_field(rest)?;
    rest.trim().is_empty().then_some((old, new))
}

fn parse_git_field(value: &str) -> Option<(String, &str)> {
    let value = value.trim_start();
    if value.starts_with('"') {
        let (unquoted, consumed) = gix_quote::ansi_c::undo(value.as_bytes().into()).ok()?;
        let rest = value.get(consumed..)?;
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        return Some((
            String::from_utf8_lossy(unquoted.as_ref()).into_owned(),
            rest,
        ));
    }
    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    (end > 0).then(|| (value[..end].to_owned(), &value[end..]))
}

fn strip_git_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_owned()
}

fn parse_range(value: &str) -> Option<(u64, usize)> {
    let (start, count) = value
        .split_once(',')
        .map_or((value, "1"), |(start, count)| (start, count));
    Some((start.parse().ok()?, count.parse().ok()?))
}
