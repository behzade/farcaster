//! Read-only Git projection for files observed in a selected session tree.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::SystemTime,
};

use crate::sessions::normalize_lexical;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum FileChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Binary,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub kind: FileChangeKind,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub observed_at: SystemTime,
    pub exists: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChangeSet {
    pub repo_root: Option<PathBuf>,
    pub files: Vec<FileChange>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullDiff {
    pub path: PathBuf,
    pub patch: String,
    pub binary: bool,
}

pub(crate) fn collect(
    project: &Path,
    observed: impl IntoIterator<Item = (PathBuf, SystemTime)>,
) -> ChangeSet {
    match collect_inner(project, observed) {
        Ok(set) => set,
        Err(error) => ChangeSet {
            unavailable: Some(error),
            ..ChangeSet::default()
        },
    }
}

fn collect_inner(
    project: &Path,
    observed: impl IntoIterator<Item = (PathBuf, SystemTime)>,
) -> Result<ChangeSet, String> {
    let root_output = git(project, &["rev-parse", "--show-toplevel"])?;
    if !root_output.status.success() {
        return Err("Changes unavailable: selected project is not a Git repository".into());
    }
    let repo_root = PathBuf::from(String::from_utf8_lossy(&root_output.stdout).trim());
    let repo_root = fs::canonicalize(&repo_root).unwrap_or_else(|_| normalize_lexical(&repo_root));
    let head = git(&repo_root, &["rev-parse", "--verify", "HEAD"])?;
    if !head.status.success() {
        return Err("Changes unavailable: repository has no HEAD yet".into());
    }

    let mut observed_at = HashMap::<PathBuf, SystemTime>::new();
    for (path, time) in observed {
        let path = if path.is_absolute() {
            normalize_lexical(&path)
        } else {
            normalize_lexical(&project.join(path))
        };
        let path = canonicalize_allow_missing(&path);
        if !path.starts_with(&repo_root) {
            continue;
        }
        observed_at
            .entry(path)
            .and_modify(|known| *known = (*known).max(time))
            .or_insert(time);
    }
    if observed_at.is_empty() {
        return Ok(ChangeSet {
            repo_root: Some(repo_root),
            ..ChangeSet::default()
        });
    }
    let mut relatives = observed_at
        .keys()
        .filter_map(|path| path.strip_prefix(&repo_root).ok().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    relatives.sort();
    relatives.dedup();

    // Rename detection needs both sides, so status the repository once and
    // intersect the typed result with observed paths below. Complete patches
    // remain on-demand and path-scoped.
    let status = git(
        &repo_root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !status.status.success() {
        return Err(stderr(&status, "git status failed"));
    }
    let statuses = parse_status(&status.stdout);
    let numstat = git(&repo_root, &["diff", "--numstat", "-z", "HEAD"])?;
    if !numstat.status.success() {
        return Err(stderr(&numstat, "git diff failed"));
    }
    let counts = parse_numstat_z(&numstat.stdout);

    let observed_relatives = relatives
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut files = Vec::new();
    for (relative, status) in statuses {
        let (mut kind, old_path) = status;
        if !observed_relatives.contains(&relative)
            && !old_path
                .as_ref()
                .is_some_and(|old| observed_relatives.contains(old))
        {
            continue;
        }
        let absolute = repo_root.join(&relative);
        let old_absolute = old_path.as_ref().map(|path| repo_root.join(path));
        let (additions, deletions, binary) = if kind == FileChangeKind::Renamed {
            merge_rename_counts(
                counts.get(&relative).copied(),
                old_path.as_ref().and_then(|old| counts.get(old)).copied(),
            )
        } else {
            counts
                .get(&relative)
                .map(|counts| (counts.additions, counts.deletions, counts.binary))
                .unwrap_or_else(|| {
                    if kind == FileChangeKind::Added {
                        untracked_counts(&absolute)
                    } else {
                        (None, None, false)
                    }
                })
        };
        if binary {
            kind = FileChangeKind::Binary;
        }
        let time = observed_at
            .get(&absolute)
            .into_iter()
            .chain(old_absolute.as_ref().and_then(|path| observed_at.get(path)))
            .copied()
            .max()
            .or_else(|| fs::metadata(&absolute).and_then(|m| m.modified()).ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        files.push(FileChange {
            path: absolute,
            old_path: old_absolute,
            kind,
            additions,
            deletions,
            observed_at: time,
            exists: repo_root.join(&relative).exists(),
        });
    }
    files.sort_by_key(|file| std::cmp::Reverse(file.observed_at));
    Ok(ChangeSet {
        repo_root: Some(repo_root),
        files,
        unavailable: None,
    })
}

pub(crate) fn load_current_path_diff(
    project: &Path,
    path: &Path,
) -> Result<(FileChange, FullDiff), String> {
    let set = collect_inner(project, [(path.to_owned(), SystemTime::now())])?;
    let file = set
        .files
        .into_iter()
        .next()
        .ok_or_else(|| "No current HEAD diff exists for this tool path".to_owned())?;
    let repo_root = set
        .repo_root
        .ok_or_else(|| "Repository root is unavailable".to_owned())?;
    let diff = load_full_diff(&repo_root, &file)?;
    Ok((file, diff))
}

pub(crate) fn load_full_diff(repo_root: &Path, file: &FileChange) -> Result<FullDiff, String> {
    let path = file
        .path
        .strip_prefix(repo_root)
        .map_err(|_| "Observed file is outside the repository".to_owned())?;
    let mut paths = vec![path.to_owned()];
    if let Some(old_path) = &file.old_path {
        let old = old_path
            .strip_prefix(repo_root)
            .map_err(|_| "Renamed source is outside the repository".to_owned())?;
        paths.push(old.to_owned());
    }
    let output = git_paths(
        repo_root,
        &["diff", "--no-ext-diff", "--binary", "HEAD"],
        &paths,
    )?;
    if !output.status.success() {
        return Err(stderr(&output, "git diff failed"));
    }
    if output.stdout.is_empty()
        && matches!(file.kind, FileChangeKind::Added | FileChangeKind::Binary)
        && file.path.exists()
    {
        return untracked_diff(repo_root, file);
    }
    let patch = match String::from_utf8(output.stdout) {
        Ok(patch) => patch,
        Err(_) => {
            return Ok(FullDiff {
                path: file.path.clone(),
                patch: format!("Binary file differs from HEAD: {}\n", file.path.display()),
                binary: true,
            });
        }
    };
    Ok(FullDiff {
        path: file.path.clone(),
        binary: file.kind == FileChangeKind::Binary || patch.contains("GIT binary patch"),
        patch,
    })
}

fn git(cwd: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|error| format!("Changes unavailable: cannot run git: {error}"))
}

fn git_paths(cwd: &Path, args: &[&str], paths: &[PathBuf]) -> Result<Output, String> {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .arg("--")
        .args(paths)
        .output()
        .map_err(|error| format!("Changes unavailable: cannot run git: {error}"))
}

fn stderr(output: &Output, fallback: &str) -> String {
    let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if error.is_empty() {
        fallback.into()
    } else {
        error
    }
}

fn parse_status(bytes: &[u8]) -> HashMap<PathBuf, (FileChangeKind, Option<PathBuf>)> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let mut result = HashMap::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        if field.len() < 4 {
            index += 1;
            continue;
        }
        let code = &field[..2];
        let path = PathBuf::from(String::from_utf8_lossy(&field[3..]).into_owned());
        if code.contains(&b'R') && index + 1 < fields.len() {
            let old = PathBuf::from(String::from_utf8_lossy(fields[index + 1]).into_owned());
            result.insert(path, (FileChangeKind::Renamed, Some(old)));
            index += 2;
            continue;
        }
        let kind = if code == b"??" || code.contains(&b'A') {
            FileChangeKind::Added
        } else if code.contains(&b'D') {
            FileChangeKind::Deleted
        } else {
            FileChangeKind::Modified
        };
        result.insert(path, (kind, None));
        index += 1;
    }
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NumstatCounts {
    additions: Option<u64>,
    deletions: Option<u64>,
    binary: bool,
    group: u64,
}

fn parse_numstat_z(bytes: &[u8]) -> HashMap<PathBuf, NumstatCounts> {
    let mut result = HashMap::new();
    let mut group = 0_u64;
    let mut fields = bytes.split(|byte| *byte == 0);
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let Some(first_tab) = header.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        let Some(second_rel) = header[first_tab + 1..]
            .iter()
            .position(|byte| *byte == b'\t')
        else {
            continue;
        };
        let second_tab = first_tab + 1 + second_rel;
        let additions = &header[..first_tab];
        let deletions = &header[first_tab + 1..second_tab];
        group = group.saturating_add(1);
        let counts = NumstatCounts {
            additions: std::str::from_utf8(additions)
                .ok()
                .and_then(|value| value.parse().ok()),
            deletions: std::str::from_utf8(deletions)
                .ok()
                .and_then(|value| value.parse().ok()),
            binary: additions == b"-" || deletions == b"-",
            group,
        };
        let inline_path = &header[second_tab + 1..];
        if !inline_path.is_empty() {
            result.insert(
                PathBuf::from(String::from_utf8_lossy(inline_path).into_owned()),
                counts,
            );
            continue;
        }
        let Some(old) = fields.next().filter(|field| !field.is_empty()) else {
            break;
        };
        let Some(new) = fields.next().filter(|field| !field.is_empty()) else {
            break;
        };
        let old = PathBuf::from(String::from_utf8_lossy(old).into_owned());
        let new = PathBuf::from(String::from_utf8_lossy(new).into_owned());
        result.insert(old, counts);
        result.insert(new, counts);
    }
    result
}

fn merge_rename_counts(
    new: Option<NumstatCounts>,
    old: Option<NumstatCounts>,
) -> (Option<u64>, Option<u64>, bool) {
    match (new, old) {
        (Some(new), Some(old)) if new.group != old.group => (
            new.additions
                .zip(old.additions)
                .map(|(new, old)| new.saturating_add(old)),
            new.deletions
                .zip(old.deletions)
                .map(|(new, old)| new.saturating_add(old)),
            new.binary || old.binary,
        ),
        (Some(counts), _) | (_, Some(counts)) => {
            (counts.additions, counts.deletions, counts.binary)
        }
        (None, None) => (None, None, false),
    }
}

fn canonicalize_allow_missing(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut suffix = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return normalize_lexical(path);
        };
        suffix.push(name.to_owned());
        let Some(parent) = existing.parent() else {
            return normalize_lexical(path);
        };
        existing = parent;
    }
    let mut result = fs::canonicalize(existing).unwrap_or_else(|_| normalize_lexical(existing));
    for component in suffix.into_iter().rev() {
        result.push(component);
    }
    normalize_lexical(&result)
}

fn untracked_counts(path: &Path) -> (Option<u64>, Option<u64>, bool) {
    match fs::read(path) {
        Ok(bytes) if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() => {
            (None, None, true)
        }
        Ok(bytes) => {
            let count = if bytes.is_empty() {
                0
            } else {
                bytes.split(|byte| *byte == b'\n').count() as u64
                    - u64::from(bytes.ends_with(b"\n"))
            };
            (Some(count), Some(0), false)
        }
        Err(_) => (None, None, false),
    }
}

fn untracked_diff(repo_root: &Path, file: &FileChange) -> Result<FullDiff, String> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["diff", "--no-index", "--binary", "--"])
        .arg(Path::new("/dev/null"))
        .arg(&file.path)
        .output()
        .map_err(|error| format!("Changes unavailable: cannot run git: {error}"))?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(stderr(&output, "git diff --no-index failed"));
    }
    let patch = match String::from_utf8(output.stdout) {
        Ok(patch) => patch,
        Err(_) => format!("Binary untracked file: {}\n", file.path.display()),
    };
    let binary = file.kind == FileChangeKind::Binary
        || patch.contains("GIT binary patch")
        || patch.starts_with("Binary untracked file:");
    Ok(FullDiff {
        path: file.path.clone(),
        patch,
        binary,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::fs;

    fn run(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        run(temp.path(), &["init", "-q"]);
        fs::write(temp.path().join("tracked.txt"), "one\n").unwrap();
        run(temp.path(), &["add", "."]);
        run(
            temp.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        );
        temp
    }

    #[test]
    fn projects_current_head_net_diff_for_observed_files_once() {
        let repo = repo();
        let path = repo.path().join("tracked.txt");
        fs::write(&path, "two\nthree\n").unwrap();
        run(repo.path(), &["add", "tracked.txt"]);
        fs::write(&path, "three\nfour\n").unwrap();
        let now = SystemTime::now();
        let set = collect(repo.path(), [(path.clone(), now), (path.clone(), now)]);
        assert_eq!(set.files.len(), 1);
        assert_eq!(set.files[0].kind, FileChangeKind::Modified);
        assert!(set.files[0].additions.unwrap() > 0);
        assert!(
            load_full_diff(set.repo_root.as_deref().unwrap(), &set.files[0])
                .unwrap()
                .patch
                .contains("+four")
        );
    }

    #[test]
    fn includes_untracked_and_deleted_but_rejects_outside_observations() {
        let repo = repo();
        let added = repo.path().join("new.txt");
        fs::write(&added, "a\nb\n").unwrap();
        let deleted = repo.path().join("tracked.txt");
        fs::remove_file(&deleted).unwrap();
        let outside = repo.path().parent().unwrap().join("outside.txt");
        let now = SystemTime::now();
        let set = collect(
            repo.path(),
            [(added.clone(), now), (deleted, now), (outside, now)],
        );
        assert_eq!(set.files.len(), 2);
        assert!(
            set.files
                .iter()
                .any(|file| file.kind == FileChangeKind::Added && file.additions == Some(2))
        );
        assert!(
            set.files
                .iter()
                .any(|file| file.kind == FileChangeKind::Deleted)
        );
    }

    #[test]
    fn reports_renames_and_binary_changes() {
        let repo = repo();
        fs::write(repo.path().join("binary.dat"), [0, 1, 2]).unwrap();
        run(repo.path(), &["add", "binary.dat"]);
        run(
            repo.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "binary",
            ],
        );
        run(repo.path(), &["mv", "tracked.txt", "renamed.txt"]);
        let old = repo.path().join("tracked.txt");
        let renamed = repo.path().join("renamed.txt");
        fs::write(&renamed, "two\n").unwrap();
        fs::write(repo.path().join("binary.dat"), [0, 3, 4]).unwrap();
        let set = collect(
            repo.path(),
            [
                (renamed.clone(), SystemTime::now()),
                (old.clone(), SystemTime::now()),
                (repo.path().join("binary.dat"), SystemTime::now()),
            ],
        );
        assert_eq!(set.files.len(), 2, "both rename sides project one row");
        let rename = set
            .files
            .iter()
            .find(|file| file.kind == FileChangeKind::Renamed)
            .expect("rename from destination observation");
        assert_eq!(
            rename.old_path.as_deref(),
            Some(canonicalize_allow_missing(&old).as_path())
        );
        assert_eq!((rename.additions, rename.deletions), (Some(1), Some(1)));
        let binary = set
            .files
            .iter()
            .find(|file| file.kind == FileChangeKind::Binary)
            .expect("binary change");
        let binary_diff =
            load_full_diff(set.repo_root.as_deref().expect("repo"), binary).expect("binary diff");
        assert!(binary_diff.binary);
        assert!(!binary_diff.patch.is_empty());

        let from_old = collect(repo.path(), [(old, SystemTime::now())]);
        assert_eq!(from_old.files.len(), 1);
        assert_eq!(from_old.files[0].kind, FileChangeKind::Renamed);
        assert_eq!(
            (from_old.files[0].additions, from_old.files[0].deletions),
            (Some(1), Some(1))
        );
    }

    #[test]
    fn non_repository_and_unborn_repository_are_truthfully_unavailable() {
        let plain = tempfile::tempdir().unwrap();
        assert!(collect(plain.path(), []).unavailable.is_some());
        run(plain.path(), &["init", "-q"]);
        assert!(
            collect(plain.path(), [])
                .unavailable
                .unwrap()
                .contains("no HEAD")
        );
    }

    #[test]
    fn untracked_binary_uses_a_complete_no_index_patch() {
        let repo = repo();
        let path = repo.path().join("untracked.bin");
        fs::write(&path, [0, 1, 2, 3]).unwrap();
        let set = collect(repo.path(), [(path, SystemTime::now())]);
        assert_eq!(set.files[0].kind, FileChangeKind::Binary);
        let diff = load_full_diff(set.repo_root.as_deref().unwrap(), &set.files[0]).unwrap();
        assert!(diff.binary);
        assert!(!diff.patch.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn untracked_diff_preserves_crlf_final_newline_and_executable_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = repo();
        let path = repo.path().join("script.sh");
        fs::write(&path, b"first\r\nlast").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        let set = collect(repo.path(), [(path, SystemTime::now())]);
        let diff = load_full_diff(set.repo_root.as_deref().unwrap(), &set.files[0]).unwrap();
        assert!(diff.patch.contains("new file mode 100755"));
        assert!(diff.patch.contains("+first\r"));
        assert!(diff.patch.contains("\\ No newline at end of file"));
    }

    #[test]
    fn complete_untracked_patch_is_not_line_limited() {
        let repo = repo();
        let path = repo.path().join("large.txt");
        fs::write(
            &path,
            (0..700)
                .map(|line| format!("line {line}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let set = collect(repo.path(), [(path, SystemTime::now())]);
        let diff = load_full_diff(set.repo_root.as_deref().unwrap(), &set.files[0]).unwrap();
        assert!(diff.patch.contains("+line 699"));
    }
}
