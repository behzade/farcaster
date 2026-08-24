//! Pure projection of successful file tools recorded by Pi sessions.

use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use crate::agent_activity::{FileMutation, FileMutationKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileChangeKind {
    Edited,
    Written,
    Mixed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullDiff {
    pub path: PathBuf,
    pub patch: String,
    pub partial: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub observed_at: SystemTime,
    pub exists: bool,
    pub operations: usize,
    pub diff: FullDiff,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChangeSet {
    pub files: Vec<FileChange>,
    pub incomplete: bool,
}

#[derive(Default)]
struct PendingFile {
    path: PathBuf,
    latest: Option<SystemTime>,
    edits: usize,
    writes: usize,
    additions: Option<u64>,
    deletions: Option<u64>,
    operations: Vec<String>,
    partial: bool,
}

pub(crate) fn collect(mutations: impl IntoIterator<Item = FileMutation>) -> ChangeSet {
    let mut mutations = mutations.into_iter().collect::<Vec<_>>();
    mutations.sort_by_key(|mutation| mutation.observed_at);
    let mut files = Vec::<PendingFile>::new();
    let mut indices = HashMap::<PathBuf, usize>::new();

    for mutation in mutations {
        let index = if let Some(index) = indices.get(&mutation.path).copied() {
            index
        } else {
            let index = files.len();
            indices.insert(mutation.path.clone(), index);
            files.push(PendingFile {
                path: mutation.path.clone(),
                additions: Some(0),
                deletions: Some(0),
                ..PendingFile::default()
            });
            index
        };
        let file = &mut files[index];
        file.latest = Some(file.latest.map_or(mutation.observed_at, |latest| {
            latest.max(mutation.observed_at)
        }));
        let operation_number = file.operations.len().saturating_add(1);
        match mutation.kind {
            FileMutationKind::Edit { patch, complete } => {
                file.edits = file.edits.saturating_add(1);
                file.partial |= !complete || patch.is_empty();
                let counts = patch_counts(&patch);
                add_counts(&mut file.additions, counts.0);
                add_counts(&mut file.deletions, counts.1);
                file.operations.push(format_operation(
                    "edit",
                    operation_number,
                    if patch.is_empty() {
                        "Recorded edit has no retained diff.\n"
                    } else {
                        &patch
                    },
                ));
            }
            FileMutationKind::Write { content } => {
                file.writes = file.writes.saturating_add(1);
                add_counts(&mut file.additions, Some(line_count(&content)));
                add_counts(&mut file.deletions, Some(0));
                file.operations.push(format_operation(
                    "write",
                    operation_number,
                    &write_patch(&content),
                ));
            }
        }
    }

    let mut files = files
        .into_iter()
        .map(|file| {
            let kind = match (file.edits > 0, file.writes > 0) {
                (true, true) => FileChangeKind::Mixed,
                (true, false) => FileChangeKind::Edited,
                (false, true) => FileChangeKind::Written,
                (false, false) => unreachable!("files are created from mutations"),
            };
            let patch = file.operations.join("\n");
            FileChange {
                path: file.path.clone(),
                kind,
                additions: file.additions,
                deletions: file.deletions,
                observed_at: file.latest.unwrap_or(SystemTime::UNIX_EPOCH),
                exists: file.path.exists(),
                operations: file.edits.saturating_add(file.writes),
                diff: FullDiff {
                    path: file.path,
                    patch,
                    partial: file.partial,
                },
            }
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|file| std::cmp::Reverse(file.observed_at));
    ChangeSet {
        files,
        incomplete: false,
    }
}

fn add_counts(total: &mut Option<u64>, next: Option<u64>) {
    *total = total
        .zip(next)
        .map(|(total, next)| total.saturating_add(next));
}

fn patch_counts(patch: &str) -> (Option<u64>, Option<u64>) {
    if patch.is_empty() {
        return (None, None);
    }
    let mut additions = 0_u64;
    let mut deletions = 0_u64;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions = additions.saturating_add(1);
        } else if line.starts_with('-') {
            deletions = deletions.saturating_add(1);
        }
    }
    (Some(additions), Some(deletions))
}

fn line_count(content: &str) -> u64 {
    if content.is_empty() {
        0
    } else {
        content.lines().count() as u64
    }
}

fn write_patch(content: &str) -> String {
    let mut patch = String::new();
    for line in content.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    patch
}

fn format_operation(kind: &str, number: usize, patch: &str) -> String {
    let mut result = format!("recorded {kind} operation {number}\n");
    result.push_str(patch);
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mutation(path: &str, seconds: u64, kind: FileMutationKind) -> FileMutation {
        FileMutation {
            path: PathBuf::from(path),
            observed_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
            kind,
        }
    }

    #[test]
    fn aggregates_recorded_operations_by_path_in_call_order() {
        let set = collect([
            mutation(
                "/project/src/lib.rs",
                3,
                FileMutationKind::Edit {
                    patch: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n-old\n+new\n".into(),
                    complete: true,
                },
            ),
            mutation(
                "/project/new.txt",
                2,
                FileMutationKind::Write {
                    content: "one\ntwo\n".into(),
                },
            ),
            mutation(
                "/project/src/lib.rs",
                4,
                FileMutationKind::Edit {
                    patch: "@@\n-new\n+final\n".into(),
                    complete: true,
                },
            ),
        ]);

        assert_eq!(set.files.len(), 2);
        let edited = &set.files[0];
        assert_eq!(edited.path, PathBuf::from("/project/src/lib.rs"));
        assert_eq!(edited.operations, 2);
        assert_eq!((edited.additions, edited.deletions), (Some(2), Some(2)));
        assert!(
            edited
                .diff
                .patch
                .find("+new")
                .expect("first edit should be present")
                < edited
                    .diff
                    .patch
                    .find("+final")
                    .expect("second edit should be present")
        );
        assert!(!edited.diff.partial);
        let written = &set.files[1];
        assert_eq!(written.kind, FileChangeKind::Written);
        assert_eq!((written.additions, written.deletions), (Some(2), Some(0)));
    }

    #[test]
    fn works_without_a_repository_and_never_reads_current_file_contents() {
        let set = collect([mutation(
            "/path/that/does/not/exist.txt",
            1,
            FileMutationKind::Edit {
                patch: "@@\n-session value\n+recorded value\n".into(),
                complete: true,
            },
        )]);
        assert_eq!(set.files.len(), 1);
        assert!(set.files[0].diff.patch.contains("+recorded value"));
        assert!(!set.files[0].diff.patch.contains("HEAD"));
    }

    #[test]
    fn marks_argument_only_edit_previews_as_partial() {
        let set = collect([mutation(
            "/project/file.txt",
            1,
            FileMutationKind::Edit {
                patch: "- before\n+ after\n".into(),
                complete: false,
            },
        )]);
        assert!(set.files[0].diff.partial);
        assert_eq!(
            (set.files[0].additions, set.files[0].deletions),
            (Some(1), Some(1))
        );
    }
}
