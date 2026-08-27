use std::{ffi::OsString, path::PathBuf, sync::Arc, time::SystemTime};

use super::{
    ChangeKind, ChangeLayer, DiffResult, DiffTarget, JujutsuIdentity, RepositoryBackend,
    RepositoryError, RepositoryKind, SnapshotIdentity, SnapshotToken, WorkingCopySnapshot, change,
    diff_result, require_complete_stdout,
};

const OPERATION_TEMPLATE: &str = "id ++ \"\\n\"";
const IDENTITY_TEMPLATE: &str = concat!(
    "json(commit_id) ++ \"\\t\" ++ ",
    "json(change_id) ++ \"\\t\" ++ ",
    "json(description.first_line()) ++ \"\\t\" ++ ",
    "json(conflict) ++ \"\\t\" ++ ",
    "json(empty) ++ \"\\n\" ++ ",
    "bookmarks.map(|bookmark| json(bookmark.name())).join(\"\\t\") ++ \"\\n\" ++ ",
    "conflicted_files.map(|file| json(file.path())).join(\"\\t\") ++ \"\\n\""
);
const STATUS_TEMPLATE: &str = concat!(
    "json(status) ++ \"\\t\" ++ ",
    "json(source.path()) ++ \"\\t\" ++ ",
    "json(target.path()) ++ \"\\t\" ++ ",
    "json(source.conflict()) ++ \"\\t\" ++ ",
    "json(target.conflict()) ++ \"\\n\""
);

pub(super) fn snapshot(
    backend: &RepositoryBackend,
) -> Result<WorkingCopySnapshot, RepositoryError> {
    // This first read snapshots the working copy. The other reads are pinned to
    // the resulting operation, so Jujutsu identity and status cannot be torn.
    let operation_id = current_operation(backend)?;
    let identity_output = run_at_operation(
        backend,
        &operation_id,
        &["log", "-r", "@", "--no-graph", "-T", IDENTITY_TEMPLATE],
        false,
    )?;
    let status_output = run_at_operation(
        backend,
        &operation_id,
        &["diff", "-r", "@", "-T", STATUS_TEMPLATE],
        false,
    )?;
    let mut identity = parse_identity(&identity_output.stdout)?;
    identity.operation_id.clone_from(&operation_id);
    let token = SnapshotToken::Jujutsu(Arc::from(operation_id));
    let mut parsed = parse_status(&status_output.stdout)?;
    for path in &identity.conflicted_paths {
        if !parsed.iter().any(|change| &change.relative_path == path) {
            parsed.push(ParsedChange {
                relative_path: path.clone(),
                original_relative_path: None,
                kind: ChangeKind::Conflict,
            });
        }
    }
    let project = backend.project_pathspec();
    let changes = parsed
        .into_iter()
        .filter_map(|parsed| scope_change_to_project(parsed, &project))
        .map(|parsed| {
            change(
                &backend.location,
                token.clone(),
                parsed.relative_path,
                parsed.original_relative_path,
                ChangeLayer::JujutsuWorkingCopy,
                parsed.kind,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkingCopySnapshot {
        location: backend.location.clone(),
        identity: SnapshotIdentity::Jujutsu(identity),
        changes,
        captured_at: SystemTime::now(),
    })
}

pub(super) fn list_project_files(
    backend: &RepositoryBackend,
) -> Result<Vec<String>, RepositoryError> {
    let operation = current_operation(backend)?;
    let output = run_at_operation(
        backend,
        &operation,
        &["file", "list", "-T", "json(path) ++ \"\\n\""],
        false,
    )?;
    let text =
        std::str::from_utf8(&output.stdout).map_err(|_| invalid("file list is not UTF-8"))?;
    let mut files = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(decode_json_string)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(PathBuf::from)
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
    let SnapshotToken::Jujutsu(expected_operation) = &target.token else {
        return Err(RepositoryError::TargetMismatch(
            "Git snapshot token used with Jujutsu".to_owned(),
        ));
    };
    if current_operation(backend)? != expected_operation.as_ref() {
        return Err(RepositoryError::StaleSnapshot);
    }
    let fileset = diff_fileset(&target)?;
    let arguments = vec![
        OsString::from("--no-pager"),
        OsString::from("--color=never"),
        OsString::from("--at-operation"),
        OsString::from(expected_operation.as_ref()),
        OsString::from("diff"),
        OsString::from("-r"),
        OsString::from("@"),
        OsString::from("--git"),
        OsString::from("--"),
        OsString::from(fileset),
    ];
    let output = backend.run_success(&arguments)?;
    require_complete_stdout(backend.executable(), &output)?;
    Ok(diff_result(
        target,
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

fn current_operation(backend: &RepositoryBackend) -> Result<String, RepositoryError> {
    let arguments = [
        "--no-pager",
        "--color=never",
        "op",
        "log",
        "-n",
        "1",
        "--no-graph",
        "-T",
        OPERATION_TEMPLATE,
    ]
    .map(OsString::from);
    let output = backend.run_success(&arguments)?;
    require_complete_stdout(backend.executable(), &output)?;
    let operation = std::str::from_utf8(&output.stdout)
        .map_err(|_| invalid("operation id is not UTF-8"))?
        .trim();
    if operation.is_empty() || !operation.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("operation id is empty or malformed"));
    }
    Ok(operation.to_owned())
}

fn run_at_operation(
    backend: &RepositoryBackend,
    operation_id: &str,
    command: &[&str],
    project_scoped: bool,
) -> Result<super::process::CommandOutput, RepositoryError> {
    let mut arguments = vec![
        OsString::from("--no-pager"),
        OsString::from("--color=never"),
        OsString::from("--at-operation"),
        OsString::from(operation_id),
    ];
    arguments.extend(command.iter().map(OsString::from));
    if project_scoped {
        arguments.push(OsString::from("--"));
        arguments.push(backend.project_pathspec().into_os_string());
    }
    let output = backend.run_success(&arguments)?;
    require_complete_stdout(backend.executable(), &output)?;
    Ok(output)
}

fn parse_identity(input: &[u8]) -> Result<JujutsuIdentity, RepositoryError> {
    let text = std::str::from_utf8(input).map_err(|_| invalid("identity is not UTF-8"))?;
    let mut lines = text.split('\n');
    let fields = lines
        .next()
        .unwrap_or_default()
        .split('\t')
        .collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(invalid("identity record does not contain five fields"));
    }
    let bookmarks = decode_json_string_list(lines.next().unwrap_or_default())?;
    let conflicted_paths = decode_json_string_list(lines.next().unwrap_or_default())?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    Ok(JujutsuIdentity {
        operation_id: String::new(),
        commit_id: decode_json_string(fields[0])?,
        change_id: decode_json_string(fields[1])?,
        description: decode_json_string(fields[2])?,
        bookmarks,
        closest_bookmarks: Vec::new(),
        ahead: 0,
        conflicted_paths,
        conflicted: parse_json_bool(fields[3])?,
        empty: parse_json_bool(fields[4])?,
    })
}

#[derive(Debug, Eq, PartialEq)]
struct ParsedChange {
    relative_path: PathBuf,
    original_relative_path: Option<PathBuf>,
    kind: ChangeKind,
}

fn parse_status(input: &[u8]) -> Result<Vec<ParsedChange>, RepositoryError> {
    let text = std::str::from_utf8(input).map_err(|_| invalid("status is not UTF-8"))?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 5 {
                return Err(invalid("status record does not contain five fields"));
            }
            let status = decode_json_string(fields[0])?;
            let source = decode_optional_json_path(fields[1])?;
            let target = decode_optional_json_path(fields[2])?;
            let relative_path = target
                .clone()
                .or_else(|| source.clone())
                .ok_or_else(|| invalid("status record has no source or target path"))?;
            let conflicted = parse_json_bool(fields[3])? || parse_json_bool(fields[4])?;
            let kind = if conflicted {
                ChangeKind::Conflict
            } else {
                jj_kind(&status)
            };
            let original_relative_path =
                if matches!(kind, ChangeKind::Renamed | ChangeKind::Copied) && source != target {
                    source
                } else {
                    None
                };
            Ok(ParsedChange {
                relative_path,
                original_relative_path,
                kind,
            })
        })
        .collect()
}

fn scope_change_to_project(
    mut change: ParsedChange,
    project: &std::path::Path,
) -> Option<ParsedChange> {
    if project == std::path::Path::new(".") {
        return Some(change);
    }
    let target_is_inside = change.relative_path.starts_with(project);
    let source_is_inside = change
        .original_relative_path
        .as_deref()
        .is_some_and(|path| path.starts_with(project));
    match (change.kind.clone(), source_is_inside, target_is_inside) {
        (ChangeKind::Renamed, true, false) => {
            change.relative_path = change.original_relative_path.take()?;
            change.kind = ChangeKind::Deleted;
            Some(change)
        }
        (ChangeKind::Renamed | ChangeKind::Copied, false, true) => {
            change.original_relative_path = None;
            change.kind = ChangeKind::Added;
            Some(change)
        }
        (ChangeKind::Copied, true, false) => None,
        (ChangeKind::Renamed | ChangeKind::Copied, true, true) => Some(change),
        (ChangeKind::Renamed | ChangeKind::Copied, false, false) => None,
        (_, _, true) => Some(change),
        (_, _, false) => None,
    }
}

fn diff_fileset(target: &DiffTarget) -> Result<String, RepositoryError> {
    let target_fileset = literal_fileset(&target.relative_path)?;
    if let Some(source) = target.original_relative_path.as_deref() {
        Ok(format!("{} | {target_fileset}", literal_fileset(source)?))
    } else {
        Ok(target_fileset)
    }
}

fn literal_fileset(path: &std::path::Path) -> Result<String, RepositoryError> {
    let path = path
        .to_str()
        .ok_or_else(|| RepositoryError::InvalidPath(path.to_path_buf()))?;
    Ok(format!("root-file:{}", encode_json_string(path)))
}

fn encode_json_string(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len().saturating_add(2));
    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{0008}' => encoded.push_str("\\b"),
            '\u{000c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _write_result = write!(encoded, "\\u{:04x}", u32::from(character));
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn decode_json_string_list(value: &str) -> Result<Vec<String>, RepositoryError> {
    value
        .split('\t')
        .filter(|field| !field.is_empty())
        .map(decode_json_string)
        .collect()
}

fn decode_optional_json_path(value: &str) -> Result<Option<PathBuf>, RepositoryError> {
    if value == "null" {
        Ok(None)
    } else {
        decode_json_string(value).map(PathBuf::from).map(Some)
    }
}

fn jj_kind(status: &str) -> ChangeKind {
    match status {
        "added" => ChangeKind::Added,
        "modified" => ChangeKind::Modified,
        "removed" => ChangeKind::Deleted,
        "renamed" => ChangeKind::Renamed,
        "copied" => ChangeKind::Copied,
        "conflict" | "conflicted" => ChangeKind::Conflict,
        other => ChangeKind::Unknown(other.to_owned()),
    }
}

fn parse_json_bool(value: &str) -> Result<bool, RepositoryError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid("invalid JSON boolean")),
    }
}

fn decode_json_string(value: &str) -> Result<String, RepositoryError> {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
    else {
        return Err(invalid("expected a JSON string"));
    };
    let mut characters = inner.chars();
    let mut decoded = String::with_capacity(inner.len());
    while let Some(character) = characters.next() {
        if character != '\\' {
            if character.is_control() {
                return Err(invalid("unescaped control character in JSON string"));
            }
            decoded.push(character);
            continue;
        }
        match characters
            .next()
            .ok_or_else(|| invalid("unterminated JSON escape"))?
        {
            '"' => decoded.push('"'),
            '\\' => decoded.push('\\'),
            '/' => decoded.push('/'),
            'b' => decoded.push('\u{0008}'),
            'f' => decoded.push('\u{000c}'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'u' => decoded.push(decode_unicode_escape(&mut characters)?),
            _ => return Err(invalid("unknown JSON escape")),
        }
    }
    Ok(decoded)
}

fn decode_unicode_escape(
    characters: &mut impl Iterator<Item = char>,
) -> Result<char, RepositoryError> {
    let first = decode_hex_quad(characters)?;
    let scalar = if (0xd800..=0xdbff).contains(&first) {
        if characters.next() != Some('\\') || characters.next() != Some('u') {
            return Err(invalid("high surrogate has no low surrogate"));
        }
        let second = decode_hex_quad(characters)?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err(invalid("invalid low surrogate"));
        }
        0x1_0000 + ((first - 0xd800) << 10) + (second - 0xdc00)
    } else if (0xdc00..=0xdfff).contains(&first) {
        return Err(invalid("unexpected low surrogate"));
    } else {
        first
    };
    char::from_u32(scalar).ok_or_else(|| invalid("invalid Unicode scalar"))
}

fn decode_hex_quad(characters: &mut impl Iterator<Item = char>) -> Result<u32, RepositoryError> {
    let mut value = 0_u32;
    for _ in 0..4 {
        let digit = characters
            .next()
            .and_then(|character| character.to_digit(16))
            .ok_or_else(|| invalid("invalid JSON Unicode escape"))?;
        value = value * 16 + digit;
    }
    Ok(value)
}

fn invalid(detail: impl Into<String>) -> RepositoryError {
    RepositoryError::InvalidOutput {
        backend: RepositoryKind::Jujutsu,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identity_and_escaped_bookmarks() {
        let identity = parse_identity(
            b"\"commit\"\t\"change\"\t\"line\\tone\"\tfalse\ttrue\n\"topic\"\t\"quote\\\"name\"\n\"conflicted.rs\"\n",
        )
        .expect("identity should parse");
        assert_eq!(identity.description, "line\tone");
        assert_eq!(identity.bookmarks, ["topic", "quote\"name"]);
        assert!(identity.closest_bookmarks.is_empty());
        assert_eq!(identity.conflicted_paths, [PathBuf::from("conflicted.rs")]);
        assert!(!identity.conflicted);
        assert!(identity.empty);
    }

    #[test]
    fn parses_status_and_preserves_rename_source() {
        let changes = parse_status(
            b"\"modified\"\t\"tab\\tname\"\t\"tab\\tname\"\tfalse\tfalse\n\"renamed\"\t\"old\"\t\"new name\"\tfalse\tfalse\n",
        )
        .expect("status should parse");
        assert_eq!(changes[0].relative_path, PathBuf::from("tab\tname"));
        assert_eq!(changes[1].kind, ChangeKind::Renamed);
        assert_eq!(
            changes[1].original_relative_path,
            Some(PathBuf::from("old"))
        );
    }

    #[test]
    fn cross_project_renames_become_scoped_additions_or_deletions() {
        let project = std::path::Path::new("project");
        let moved_out = scope_change_to_project(
            ParsedChange {
                relative_path: PathBuf::from("outside/file.rs"),
                original_relative_path: Some(PathBuf::from("project/file.rs")),
                kind: ChangeKind::Renamed,
            },
            project,
        )
        .expect("in-project deletion");
        assert_eq!(moved_out.relative_path, PathBuf::from("project/file.rs"));
        assert_eq!(moved_out.original_relative_path, None);
        assert_eq!(moved_out.kind, ChangeKind::Deleted);

        let moved_in = scope_change_to_project(
            ParsedChange {
                relative_path: PathBuf::from("project/file.rs"),
                original_relative_path: Some(PathBuf::from("outside/file.rs")),
                kind: ChangeKind::Renamed,
            },
            project,
        )
        .expect("in-project addition");
        assert_eq!(moved_in.relative_path, PathBuf::from("project/file.rs"));
        assert_eq!(moved_in.original_relative_path, None);
        assert_eq!(moved_in.kind, ChangeKind::Added);
    }

    #[test]
    fn literal_filesets_do_not_interpret_path_operators() {
        assert_eq!(
            literal_fileset(std::path::Path::new("a|b")).expect("fileset"),
            "root-file:\"a|b\""
        );
    }

    #[test]
    fn decodes_surrogate_pairs() {
        assert_eq!(
            decode_json_string("\"smile: \\ud83d\\ude00\"").expect("JSON should parse"),
            "smile: 😀"
        );
    }
}
