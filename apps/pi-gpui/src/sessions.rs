//! Read-only, bounded discovery of Pi v3 session metadata.

use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufRead as _, BufReader},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use serde_json::Value;

const MAX_CANDIDATES: usize = 2_000;
const MAX_DIRECTORIES: usize = 2_000;
const MAX_DEPTH: usize = 6;
const MAX_LINES_PER_FILE: usize = 10_000;
const MAX_SEARCH_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionSummary {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub first_user_message: String,
    pub timestamp: String,
    pub parent_session: Option<String>,
    pub modified: SystemTime,
    pub message_count: usize,
    search: String,
}

pub(crate) fn configured_session_root() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let agent = std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from);
    let session = std::env::var_os("PI_CODING_AGENT_SESSION_DIR").map(PathBuf::from);
    session_root_from(home.as_deref(), agent.as_deref(), session.as_deref())
}

pub(crate) fn session_root_from(
    home: Option<&Path>,
    agent: Option<&Path>,
    session: Option<&Path>,
) -> Result<PathBuf, String> {
    let cwd = std::env::current_dir()
        .map_err(|error| format!("resolve current directory for Pi sessions: {error}"))?;
    session_root_from_at(&cwd, home, agent, session)
}

fn session_root_from_at(
    cwd: &Path,
    home: Option<&Path>,
    agent: Option<&Path>,
    session: Option<&Path>,
) -> Result<PathBuf, String> {
    let configured = if let Some(session) = session {
        session.to_path_buf()
    } else if let Some(agent) = agent {
        agent.join("sessions")
    } else {
        home.map(|home| home.join(".pi/agent/sessions"))
            .ok_or_else(|| {
                "HOME is not set and no Pi session directory override is configured".to_owned()
            })?
    };
    Ok(if configured.is_absolute() {
        normalize_lexical(&configured)
    } else {
        normalize_lexical(&cwd.join(configured))
    })
}

pub(crate) fn discover(project: &Path, query: &str) -> Result<Vec<SessionSummary>, String> {
    discover_in(&configured_session_root()?, project, query)
}

pub(crate) fn discover_in(
    root: &Path,
    project: &Path,
    query: &str,
) -> Result<Vec<SessionSummary>, String> {
    let project = normalize_existing(project)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let mut directories_seen = 0_usize;
    let mut queue = VecDeque::from([(root.to_path_buf(), 0_usize)]);
    while let Some((directory, depth)) = queue.pop_front() {
        if directories_seen >= MAX_DIRECTORIES {
            break;
        }
        directories_seen = directories_seen.saturating_add(1);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if candidates.len() >= MAX_CANDIDATES {
                break;
            }
            let path = entry.path();
            if path.is_dir() && depth < MAX_DEPTH {
                queue.push_back((path, depth + 1));
            } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                candidates.push(path);
            }
        }
    }
    let needle = query.to_lowercase();
    let mut sessions = candidates
        .into_iter()
        .filter_map(|path| parse_candidate(&path, &project).ok().flatten())
        .filter(|session| needle.is_empty() || session.search.contains(&needle))
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.timestamp.cmp(&left.timestamp))
    });
    Ok(sessions)
}

fn parse_candidate(path: &Path, project: &Path) -> Result<Option<SessionSummary>, String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    if reader
        .read_until(b'\n', &mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Ok(None);
    }
    trim_frame(&mut line);
    let header: Value = match serde_json::from_slice(&line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return Ok(None);
    }
    let cwd = header
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let normalized_cwd =
        normalize_existing(Path::new(cwd)).unwrap_or_else(|_| normalize_lexical(Path::new(cwd)));
    if normalized_cwd != project {
        return Ok(None);
    }
    let id = header
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if id.is_empty() {
        return Ok(None);
    }
    let timestamp = header
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let parent_session = header
        .get("parentSession")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut name = None;
    let mut first_user_message = None;
    let mut message_count = 0_usize;
    let mut search = String::new();
    for _ in 0..MAX_LINES_PER_FILE {
        line.clear();
        if reader.read_until(b'\n', &mut line).unwrap_or(0) == 0 {
            break;
        }
        trim_frame(&mut line);
        let Ok(entry) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("session_info") => {
                name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            }
            Some("message") => {
                message_count = message_count.saturating_add(1);
                if let Some(message) = entry.get("message") {
                    let text = visible_user_text(message);
                    if !text.is_empty()
                        && message.get("role").and_then(Value::as_str) == Some("user")
                        && first_user_message.is_none()
                    {
                        first_user_message = Some(text.clone());
                    }
                    append_bounded(&mut search, &text);
                }
            }
            Some("custom_message")
                if entry.get("display").and_then(Value::as_bool) == Some(true) =>
            {
                append_bounded(
                    &mut search,
                    entry
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
            }
            _ => {}
        }
    }
    let first_user_message = first_user_message.unwrap_or_default();
    let title = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback_title(&first_user_message, &timestamp));
    append_bounded(&mut search, &title);
    Ok(Some(SessionSummary {
        id,
        path: path
            .canonicalize()
            .map(|path| normalize_lexical(&path))
            .map_err(|error| format!("resolve session {}: {error}", path.display()))?,
        title,
        first_user_message,
        timestamp,
        parent_session,
        modified,
        message_count,
        search: search.to_lowercase(),
    }))
}

fn visible_user_text(message: &Value) -> String {
    let Some(content) = message.get("content") else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    (block.get("type").and_then(Value::as_str) == Some("text"))
                        .then(|| block.get("text").and_then(Value::as_str))
                        .flatten()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn fallback_title(message: &str, timestamp: &str) -> String {
    let title = message
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        format!("Session {}", timestamp.get(..10).unwrap_or("unknown"))
    } else {
        title
    }
}

fn append_bounded(target: &mut String, value: &str) {
    if target.len() >= MAX_SEARCH_BYTES {
        return;
    }
    target.push(' ');
    for character in value.chars() {
        if target.len() + character.len_utf8() > MAX_SEARCH_BYTES {
            break;
        }
        target.push(character);
    }
}

fn trim_frame(frame: &mut Vec<u8>) {
    if frame.last() == Some(&b'\n') {
        frame.pop();
    }
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
}

fn normalize_existing(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map(|path| normalize_lexical(&path))
        .map_err(|error| format!("resolve project {}: {error}", path.display()))
}

pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{error::Error, fs};
    use tempfile::tempdir;

    type TestResult = Result<(), Box<dyn Error>>;

    fn session(
        root: &Path,
        file: &str,
        cwd: &Path,
        name: Option<&str>,
        message: &str,
    ) -> TestResult {
        let directory = root.join("custom/nested");
        fs::create_dir_all(&directory)?;
        let mut lines = vec![
            serde_json::json!({"type":"session","version":3,"id":file,"timestamp":"2026-01-02T00:00:00Z","cwd":cwd}),
        ];
        lines.push(serde_json::json!({"type":"unknown","data":true}));
        lines.push(
            serde_json::json!({"type":"message","message":{"role":"user","content":message}}),
        );
        if let Some(name) = name {
            lines.push(serde_json::json!({"type":"session_info","name":name}));
        }
        fs::write(
            directory.join(format!("{file}.jsonl")),
            lines
                .into_iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        Ok(())
    }

    #[test]
    fn override_resolution_order_is_explicit() {
        let cwd = Path::new("/work");
        assert_eq!(
            session_root_from_at(
                cwd,
                Some(Path::new("/h")),
                Some(Path::new("/a")),
                Some(Path::new("/s"))
            ),
            Ok(PathBuf::from("/s"))
        );
        assert_eq!(
            session_root_from_at(cwd, Some(Path::new("/h")), Some(Path::new("/a")), None),
            Ok(PathBuf::from("/a/sessions"))
        );
        assert_eq!(
            session_root_from_at(cwd, Some(Path::new("/h")), None, None),
            Ok(PathBuf::from("/h/.pi/agent/sessions"))
        );
        assert_eq!(
            session_root_from_at(cwd, None, None, Some(Path::new("relative/sessions"))),
            Ok(PathBuf::from("/work/relative/sessions"))
        );
    }

    #[test]
    fn discovers_exact_cwd_and_name_or_message_fallback() -> TestResult {
        let root = tempdir()?;
        let project = tempdir()?;
        let other = tempdir()?;
        session(
            root.path(),
            "named",
            project.path(),
            Some("Named run"),
            "first text",
        )?;
        session(
            root.path(),
            "fallback",
            project.path(),
            None,
            "A useful fallback title continues",
        )?;
        session(root.path(), "other", other.path(), Some("Wrong"), "hidden")?;
        let sessions = discover_in(root.path(), project.path(), "")?;
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|item| item.path.is_absolute()));
        assert!(sessions.iter().any(|item| item.title == "Named run"));
        assert!(
            sessions
                .iter()
                .any(|item| item.title.starts_with("A useful fallback"))
        );
        Ok(())
    }

    #[test]
    fn search_is_case_insensitive_and_malformed_entries_do_not_poison() -> TestResult {
        let root = tempdir()?;
        let project = tempdir()?;
        session(root.path(), "one", project.path(), None, "Alpha Beta")?;
        let path = root.path().join("custom/nested/one.jsonl");
        let mut content = fs::read_to_string(&path)?;
        content.push_str("\n{broken\n");
        fs::write(path, content)?;
        assert_eq!(discover_in(root.path(), project.path(), "bEtA")?.len(), 1);
        Ok(())
    }
}
