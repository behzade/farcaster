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
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub observed_at: SystemTime,
    pub partial: bool,
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
        match mutation.kind {
            FileMutationKind::Edit { patch, complete } => {
                file.edits = file.edits.saturating_add(1);
                file.partial |= !complete || patch.is_empty();
                let (additions, deletions) = patch_counts(&patch);
                add_counts(&mut file.additions, additions);
                add_counts(&mut file.deletions, deletions);
            }
            FileMutationKind::Write { content } => {
                file.writes = file.writes.saturating_add(1);
                add_counts(&mut file.additions, Some(line_count(&content)));
                add_counts(&mut file.deletions, Some(0));
            }
        }
    }

    let mut files = files
        .into_iter()
        .map(|file| FileChange {
            path: file.path,
            kind: match (file.edits > 0, file.writes > 0) {
                (true, true) => FileChangeKind::Mixed,
                (true, false) => FileChangeKind::Edited,
                (false, true) => FileChangeKind::Written,
                (false, false) => unreachable!("files are created from mutations"),
            },
            additions: file.additions,
            deletions: file.deletions,
            observed_at: file.latest.unwrap_or(SystemTime::UNIX_EPOCH),
            partial: file.partial,
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
    fn aggregates_recorded_operations_by_path() {
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
        assert_eq!((edited.additions, edited.deletions), (Some(2), Some(2)));
        assert!(!edited.partial);
        let written = &set.files[1];
        assert_eq!(written.kind, FileChangeKind::Written);
        assert_eq!((written.additions, written.deletions), (Some(2), Some(0)));
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
        assert!(set.files[0].partial);
        assert_eq!(
            (set.files[0].additions, set.files[0].deletions),
            (Some(1), Some(1))
        );
    }
}
