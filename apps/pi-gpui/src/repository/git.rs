use std::{ffi::OsString, path::PathBuf, sync::Arc, time::SystemTime};

use super::{
    ChangeKind, ChangeLayer, DiffResult, DiffTarget, GitIdentity, RepositoryBackend,
    RepositoryError, RepositoryKind, SnapshotIdentity, SnapshotToken, WorkingCopySnapshot, change,
    command_failed, diff_result, require_complete_stdout,
};

pub(super) fn snapshot(
    backend: &RepositoryBackend,
) -> Result<WorkingCopySnapshot, RepositoryError> {
    let output = status_output(backend)?;
    let token = SnapshotToken::Git(Arc::from(output.stdout.clone()));
    let (mut identity, parsed) = parse_status(&output.stdout)?;
    enrich_detached_identity(backend, &mut identity);
    let changes = parsed
        .into_iter()
        .map(|parsed| {
            change(
                &backend.location,
                token.clone(),
                parsed.relative_path,
                parsed.original_relative_path,
                parsed.layer,
                parsed.kind,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkingCopySnapshot {
        location: backend.location.clone(),
        identity: SnapshotIdentity::Git(identity),
        changes,
        captured_at: SystemTime::now(),
    })
}

pub(super) fn list_project_files(
    backend: &RepositoryBackend,
) -> Result<Vec<String>, RepositoryError> {
    let mut arguments = [
        "--no-pager",
        "--literal-pathspecs",
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
    ]
    .map(OsString::from)
    .to_vec();
    arguments.push(backend.project_pathspec().into_os_string());
    let output = backend.run_success(&arguments)?;
    require_complete_stdout(backend.executable(), &output)?;
    let mut files = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(path_from_bytes)
        .filter_map(|path| backend.project_relative_path(&path))
        .filter_map(|path| path.into_os_string().into_string().ok())
        .collect::<Vec<_>>();
    files.sort_unstable();
    files.dedup();
    Ok(files)
}

pub(super) fn load_diff(
    backend: &RepositoryBackend,
    target: DiffTarget,
) -> Result<DiffResult, RepositoryError> {
    let SnapshotToken::Git(expected_status) = &target.token else {
        return Err(RepositoryError::TargetMismatch(
            "Jujutsu snapshot token used with Git".to_owned(),
        ));
    };
    let current = status_output(backend)?;
    if current.stdout.as_slice() != expected_status.as_ref() {
        return Err(RepositoryError::StaleSnapshot);
    }

    let mut arguments = match target.layer {
        ChangeLayer::GitIndex => diff_arguments(true),
        ChangeLayer::GitWorkingTree | ChangeLayer::GitConflict => diff_arguments(false),
        ChangeLayer::GitUntracked => untracked_diff_arguments(),
        ChangeLayer::JujutsuWorkingCopy => {
            return Err(RepositoryError::TargetMismatch(
                "Jujutsu target used with Git".to_owned(),
            ));
        }
    };
    if let Some(original) = &target.original_relative_path {
        arguments.push(original.as_os_str().to_os_string());
    }
    arguments.push(target.relative_path.as_os_str().to_os_string());
    let output = backend.run(&arguments)?;
    let success = output.status.success()
        || (target.layer == ChangeLayer::GitUntracked && output.status.code() == Some(1));
    if !success {
        return Err(command_failed(backend.executable(), &output));
    }
    require_complete_stdout(backend.executable(), &output)?;
    if status_output(backend)?.stdout.as_slice() != expected_status.as_ref() {
        return Err(RepositoryError::StaleSnapshot);
    }
    Ok(diff_result(
        target,
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

fn status_output(
    backend: &RepositoryBackend,
) -> Result<super::process::CommandOutput, RepositoryError> {
    let mut arguments = [
        "--no-pager",
        "--no-optional-locks",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--ahead-behind",
        "--untracked-files=all",
        "--",
    ]
    .map(OsString::from)
    .to_vec();
    arguments.push(backend.project_pathspec().into_os_string());
    let output = backend.run_success(&arguments)?;
    require_complete_stdout(backend.executable(), &output)?;
    Ok(output)
}

fn diff_arguments(staged: bool) -> Vec<OsString> {
    let mut arguments = [
        "--no-pager",
        "--no-optional-locks",
        "--literal-pathspecs",
        "-c",
        "core.fsmonitor=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
    ]
    .map(OsString::from)
    .to_vec();
    if staged {
        arguments.push(OsString::from("--cached"));
    }
    arguments.push(OsString::from("--"));
    arguments
}

fn untracked_diff_arguments() -> Vec<OsString> {
    vec![
        OsString::from("--no-pager"),
        OsString::from("diff"),
        OsString::from("--no-index"),
        OsString::from("--no-color"),
        OsString::from("--no-ext-diff"),
        OsString::from("--no-textconv"),
        OsString::from("--"),
        OsString::from(null_device()),
    ]
}

#[cfg(unix)]
fn null_device() -> &'static str {
    "/dev/null"
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[derive(Debug, Eq, PartialEq)]
struct ParsedChange {
    relative_path: PathBuf,
    original_relative_path: Option<PathBuf>,
    layer: ChangeLayer,
    kind: ChangeKind,
}

fn enrich_detached_identity(backend: &RepositoryBackend, identity: &mut GitIdentity) {
    if identity.branch.is_some() || identity.head_oid.is_none() {
        return;
    }
    let arguments = [
        "--no-pager",
        "--no-optional-locks",
        "for-each-ref",
        "--format=%(refname:short)%09%(ahead-behind:HEAD)",
        "refs/heads",
    ]
    .map(OsString::from);
    if let Ok(output) = backend.run(&arguments)
        && output.status.success()
        && let Some((name, ref_ahead, ref_behind)) = parse_nearest_branch(&output.stdout)
    {
        identity.nearest_branch = Some(name);
        identity.behind = ref_ahead;
        identity.ahead = ref_behind;
    }
}

fn parse_nearest_branch(input: &[u8]) -> Option<(String, u64, u64)> {
    let text = std::str::from_utf8(input).ok()?;
    let mut nearest = None::<(String, u64, u64)>;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Some((name, counts)) = line.split_once('\t').filter(|(name, _)| !name.is_empty())
        else {
            continue;
        };
        let Some((ahead, behind)) = counts.split_once(' ') else {
            continue;
        };
        let Ok(ahead) = ahead.parse::<u64>() else {
            continue;
        };
        let Ok(behind) = behind.parse::<u64>() else {
            continue;
        };
        let closer = nearest
            .as_ref()
            .is_none_or(|(_, current_ahead, current_behind)| {
                let next = ahead.saturating_add(behind);
                let current = current_ahead.saturating_add(*current_behind);
                next < current || (next == current && behind < *current_behind)
            });
        if closer {
            nearest = Some((name.to_owned(), ahead, behind));
        }
    }
    nearest
}

fn parse_status(input: &[u8]) -> Result<(GitIdentity, Vec<ParsedChange>), RepositoryError> {
    let records = input.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut identity = GitIdentity::default();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if record.is_empty() {
            continue;
        }
        match record.first().copied() {
            Some(b'#') => parse_header(record, &mut identity)?,
            Some(b'1') => {
                let fields = split_fields(record, 9, "ordinary entry")?;
                push_xy_changes(
                    &mut changes,
                    parse_xy(fields[1])?,
                    path_from_bytes(fields[8]),
                    None,
                );
            }
            Some(b'2') => {
                let fields = split_fields(record, 10, "rename/copy entry")?;
                let original = records
                    .get(index)
                    .copied()
                    .ok_or_else(|| invalid("rename/copy entry has no original path"))?;
                index += 1;
                push_xy_changes(
                    &mut changes,
                    parse_xy(fields[1])?,
                    path_from_bytes(fields[9]),
                    Some(path_from_bytes(original)),
                );
            }
            Some(b'u') => {
                let fields = split_fields(record, 11, "unmerged entry")?;
                changes.push(ParsedChange {
                    relative_path: path_from_bytes(fields[10]),
                    original_relative_path: None,
                    layer: ChangeLayer::GitConflict,
                    kind: ChangeKind::Conflict,
                });
            }
            Some(b'?') if record.get(1) == Some(&b' ') => changes.push(ParsedChange {
                relative_path: path_from_bytes(&record[2..]),
                original_relative_path: None,
                layer: ChangeLayer::GitUntracked,
                kind: ChangeKind::Untracked,
            }),
            Some(b'!') => {}
            _ => return Err(invalid("unknown porcelain-v2 record type")),
        }
    }
    Ok((identity, changes))
}

fn parse_header(record: &[u8], identity: &mut GitIdentity) -> Result<(), RepositoryError> {
    let Some(header) = record.strip_prefix(b"# ") else {
        return Err(invalid("malformed branch header"));
    };
    let mut fields = header.splitn(2, |byte| *byte == b' ');
    let key = fields.next().unwrap_or_default();
    let value = fields.next().unwrap_or_default();
    match key {
        b"branch.oid" if value != b"(initial)" => {
            identity.head_oid = Some(String::from_utf8_lossy(value).into_owned());
        }
        b"branch.head" if value != b"(detached)" => {
            identity.branch = Some(String::from_utf8_lossy(value).into_owned());
        }
        b"branch.upstream" => {
            identity.upstream = Some(String::from_utf8_lossy(value).into_owned());
        }
        b"branch.ab" => {
            let value = String::from_utf8_lossy(value);
            for count in value.split_ascii_whitespace() {
                if let Some(ahead) = count.strip_prefix('+') {
                    identity.ahead = ahead.parse().map_err(|_| invalid("invalid ahead count"))?;
                } else if let Some(behind) = count.strip_prefix('-') {
                    identity.behind = behind
                        .parse()
                        .map_err(|_| invalid("invalid behind count"))?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn split_fields<'a>(
    record: &'a [u8],
    count: usize,
    kind: &str,
) -> Result<Vec<&'a [u8]>, RepositoryError> {
    let fields = record
        .splitn(count, |byte| *byte == b' ')
        .collect::<Vec<_>>();
    if fields.len() == count {
        Ok(fields)
    } else {
        Err(invalid(format!("malformed {kind}")))
    }
}

fn parse_xy(value: &[u8]) -> Result<(u8, u8), RepositoryError> {
    if value.len() == 2 {
        Ok((value[0], value[1]))
    } else {
        Err(invalid("invalid XY status"))
    }
}

fn push_xy_changes(
    changes: &mut Vec<ParsedChange>,
    (index_status, worktree_status): (u8, u8),
    path: PathBuf,
    original: Option<PathBuf>,
) {
    if index_status != b'.' {
        let kind = git_kind(index_status);
        changes.push(ParsedChange {
            relative_path: path.clone(),
            original_relative_path: rename_source(&kind, &original),
            layer: ChangeLayer::GitIndex,
            kind,
        });
    }
    if worktree_status != b'.' {
        let kind = git_kind(worktree_status);
        changes.push(ParsedChange {
            relative_path: path,
            original_relative_path: rename_source(&kind, &original),
            layer: ChangeLayer::GitWorkingTree,
            kind,
        });
    }
}

fn rename_source(kind: &ChangeKind, original: &Option<PathBuf>) -> Option<PathBuf> {
    matches!(kind, ChangeKind::Renamed | ChangeKind::Copied)
        .then(|| original.clone())
        .flatten()
}

fn git_kind(status: u8) -> ChangeKind {
    match status {
        b'A' => ChangeKind::Added,
        b'M' => ChangeKind::Modified,
        b'D' => ChangeKind::Deleted,
        b'R' => ChangeKind::Renamed,
        b'C' => ChangeKind::Copied,
        b'T' => ChangeKind::TypeChanged,
        b'U' => ChangeKind::Conflict,
        other => ChangeKind::Unknown(char::from(other).to_string()),
    }
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn invalid(detail: impl Into<String>) -> RepositoryError {
    RepositoryError::InvalidOutput {
        backend: RepositoryKind::Git,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_separate_layers_renames_untracked_and_conflicts() {
        let input = b"# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 MM N... 100644 100644 100644 aaaaaa bbbbbb src/lib.rs\0\
2 R. N... 100644 100644 100644 aaaaaa bbbbbb R100 new name.rs\0old.rs\0\
? untracked file\0\
u UU N... 100644 100644 100644 100644 aaaaaa bbbbbb cccccc conflict.txt\0";
        let (identity, changes) = parse_status(input).expect("status should parse");
        assert_eq!((identity.ahead, identity.behind), (2, 1));
        assert_eq!(changes.len(), 5);
        assert_eq!(changes[0].layer, ChangeLayer::GitIndex);
        assert_eq!(changes[1].layer, ChangeLayer::GitWorkingTree);
        assert_eq!(changes[2].kind, ChangeKind::Renamed);
        assert_eq!(
            changes[2].original_relative_path,
            Some(PathBuf::from("old.rs"))
        );
        assert_eq!(changes[3].kind, ChangeKind::Untracked);
        assert_eq!(changes[4].kind, ChangeKind::Conflict);
    }

    #[test]
    fn nearest_detached_branch_is_the_closest_local_tip() {
        let (name, ahead, behind) =
            parse_nearest_branch(b"old\t0 20\nfeature\t3 0\nmain\t0 2\n").expect("nearest branch");
        assert_eq!((name.as_str(), ahead, behind), ("main", 0, 2));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_nul_paths() {
        use std::os::unix::ffi::OsStrExt as _;

        let (_, changes) = parse_status(b"? invalid-\xff-name\0").expect("status should parse");
        assert_eq!(
            changes[0].relative_path.as_os_str().as_bytes(),
            b"invalid-\xff-name"
        );
    }
}
