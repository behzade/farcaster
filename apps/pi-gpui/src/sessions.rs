//! Read-only, bounded discovery of Pi v3 session metadata.

use std::{
    collections::{HashMap, HashSet, VecDeque},
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct UsageSummary {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
    pub cost_micros: u64,
}

impl UsageSummary {
    pub(crate) fn add(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.total = self.total.saturating_add(other.total);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
    }
}

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
    pub usage: UsageSummary,
    search: String,
}

pub(crate) fn root_sessions(sessions: &[SessionSummary]) -> Vec<&SessionSummary> {
    sessions
        .iter()
        .filter(|session| session.parent_session.is_none())
        .collect()
}

pub(crate) fn root_session_for_path<'a>(
    sessions: &'a [SessionSummary],
    selected: Option<&Path>,
) -> Option<&'a SessionSummary> {
    let by_id = sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let mut current = sessions
        .iter()
        .find(|session| selected == Some(session.path.as_path()))?;
    let mut seen = HashSet::new();
    while seen.insert(current.id.as_str()) {
        let Some(parent) = current.parent_session.as_deref() else {
            break;
        };
        let Some(parent) = by_id.get(parent) else {
            break;
        };
        current = *parent;
    }
    Some(current)
}

pub(crate) fn descendant_sessions<'a>(
    sessions: &'a [SessionSummary],
    root_id: &str,
) -> Vec<(&'a SessionSummary, usize)> {
    let mut by_parent: HashMap<&str, Vec<&SessionSummary>> = HashMap::new();
    for session in sessions {
        if let Some(parent) = session.parent_session.as_deref() {
            by_parent.entry(parent).or_default().push(session);
        }
    }
    let mut stack = by_parent
        .get(root_id)
        .into_iter()
        .flatten()
        .rev()
        .map(|session| (*session, 1_usize))
        .collect::<Vec<_>>();
    let mut descendants = Vec::new();
    let mut seen = HashSet::new();
    while let Some((session, depth)) = stack.pop() {
        if !seen.insert(session.id.as_str()) {
            continue;
        }
        descendants.push((session, depth));
        if let Some(children) = by_parent.get(session.id.as_str()) {
            stack.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (*child, depth.saturating_add(1))),
            );
        }
    }
    descendants
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

/// Read the visible, active branch of a session without starting Pi.
pub(crate) fn load_history(path: &Path) -> Result<Vec<Value>, String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut entries = Vec::new();
    for _ in 0..MAX_LINES_PER_FILE {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read {}: {error}", path.display()))?
            == 0
        {
            break;
        }
        trim_frame(&mut line);
        if let Ok(entry) = serde_json::from_slice::<Value>(&line)
            && entry.get("type").and_then(Value::as_str) != Some("session")
        {
            entries.push(entry);
        }
    }
    Ok(project_history(&entries))
}

fn project_history(entries: &[Value]) -> Vec<Value> {
    let by_id = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id, entry))
        })
        .collect::<HashMap<_, _>>();
    let Some(mut current) = entries.last() else {
        return Vec::new();
    };
    let mut branch = Vec::new();
    let mut seen = HashSet::new();
    while let Some(id) = current.get("id").and_then(Value::as_str) {
        if !seen.insert(id) {
            break;
        }
        branch.push(current);
        let Some(parent) = current.get("parentId").and_then(Value::as_str) else {
            break;
        };
        let Some(entry) = by_id.get(parent) else {
            break;
        };
        current = entry;
    }
    branch.reverse();

    let context = if let Some((index, compaction)) = branch
        .iter()
        .enumerate()
        .rev()
        .find(|(_, entry)| entry.get("type").and_then(Value::as_str) == Some("compaction"))
    {
        let first_kept = compaction.get("firstKeptEntryId").and_then(Value::as_str);
        let mut projected = vec![*compaction];
        if let Some(first_kept) = first_kept
            && let Some(kept_index) = branch[..index]
                .iter()
                .position(|entry| entry.get("id").and_then(Value::as_str) == Some(first_kept))
        {
            projected.extend_from_slice(&branch[kept_index..index]);
        }
        projected.extend_from_slice(&branch[index + 1..]);
        projected
    } else {
        branch
    };

    context.into_iter().filter_map(entry_message).collect()
}

fn entry_message(entry: &Value) -> Option<Value> {
    match entry.get("type").and_then(Value::as_str)? {
        "message" => entry.get("message").cloned(),
        "custom_message" => Some(json_object([
            ("role", Value::String("custom".into())),
            (
                "customType",
                entry.get("customType").cloned().unwrap_or(Value::Null),
            ),
            (
                "content",
                entry.get("content").cloned().unwrap_or(Value::Null),
            ),
            (
                "display",
                entry.get("display").cloned().unwrap_or(Value::Bool(true)),
            ),
        ])),
        "branch_summary" => Some(json_object([
            ("role", Value::String("branchSummary".into())),
            (
                "summary",
                entry.get("summary").cloned().unwrap_or(Value::Null),
            ),
        ])),
        "compaction" => Some(json_object([
            ("role", Value::String("compactionSummary".into())),
            (
                "summary",
                entry.get("summary").cloned().unwrap_or(Value::Null),
            ),
        ])),
        _ => None,
    }
}

fn json_object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
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
    let mut sessions = candidates
        .into_iter()
        .filter_map(|path| parse_candidate(&path, &project).ok().flatten())
        .collect::<Vec<_>>();
    let needle = query.to_lowercase();
    if !needle.is_empty() {
        let by_id = sessions
            .iter()
            .map(|session| (session.id.as_str(), session))
            .collect::<HashMap<_, _>>();
        let mut included = sessions
            .iter()
            .filter(|session| session.search.contains(&needle))
            .map(|session| session.id.clone())
            .collect::<HashSet<_>>();
        let matches = included.clone();
        for id in matches {
            let mut current = by_id.get(id.as_str()).copied();
            let mut seen = HashSet::new();
            while let Some(session) = current {
                if !seen.insert(session.id.as_str()) {
                    break;
                }
                included.insert(session.id.clone());
                current = session
                    .parent_session
                    .as_deref()
                    .and_then(|parent| by_id.get(parent).copied());
            }
        }
        let by_parent = sessions.iter().fold(
            HashMap::<&str, Vec<&str>>::new(),
            |mut children, session| {
                if let Some(parent) = session.parent_session.as_deref() {
                    children
                        .entry(parent)
                        .or_default()
                        .push(session.id.as_str());
                }
                children
            },
        );
        let mut stack = sessions
            .iter()
            .filter(|session| session.parent_session.is_none() && included.contains(&session.id))
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>();
        let mut expanded = HashSet::new();
        while let Some(parent) = stack.pop() {
            if !expanded.insert(parent) {
                continue;
            }
            if let Some(children) = by_parent.get(parent) {
                for child in children {
                    included.insert((*child).to_owned());
                    stack.push(*child);
                }
            }
        }
        sessions.retain(|session| included.contains(&session.id));
    }
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
        .map(str::to_owned)
        .or_else(|| parent_session_from_path(path));
    let mut name = None;
    let mut first_user_message = None;
    let mut message_count = 0_usize;
    let mut usage = UsageSummary::default();
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
                    usage.add(message_usage(message));
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
        usage,
        search: search.to_lowercase(),
    }))
}

fn message_usage(message: &Value) -> UsageSummary {
    let Some(usage) = message.get("usage") else {
        return UsageSummary::default();
    };
    let input = u64_field(usage, "input");
    let output = u64_field(usage, "output");
    let cache_read = u64_field(usage, "cacheRead");
    let cache_write = u64_field(usage, "cacheWrite");
    let total = u64_field(usage, "totalTokens").max(
        input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write),
    );
    let cost_micros = usage
        .get("cost")
        .and_then(|cost| cost.get("total"))
        .and_then(Value::as_f64)
        .filter(|cost| cost.is_finite() && *cost > 0.0)
        .map_or(0, |cost| (cost * 1_000_000.0).round() as u64);
    UsageSummary {
        input,
        output,
        cache_read,
        cache_write,
        total,
        cost_micros,
    }
}

fn u64_field(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn parent_session_from_path(path: &Path) -> Option<String> {
    for directory in path.parent()?.ancestors() {
        let mut root_name = directory.file_name()?.to_os_string();
        root_name.push(".jsonl");
        let root_path = directory.with_file_name(root_name);
        let Ok(file) = File::open(root_path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let Ok(read) = reader.read_until(b'\n', &mut line) else {
            continue;
        };
        if read == 0 {
            continue;
        }
        trim_frame(&mut line);
        let Ok(header) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if header.get("type").and_then(Value::as_str) == Some("session")
            && let Some(id) = header.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            return Some(id.to_owned());
        }
    }
    None
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
        session_with_parent(root, file, cwd, name, message, None)
    }

    fn session_with_parent(
        root: &Path,
        file: &str,
        cwd: &Path,
        name: Option<&str>,
        message: &str,
        parent: Option<&str>,
    ) -> TestResult {
        let directory = root.join("custom/nested");
        fs::create_dir_all(&directory)?;
        let mut lines = vec![
            serde_json::json!({"type":"session","version":3,"id":file,"timestamp":"2026-01-02T00:00:00Z","cwd":cwd,"parentSession":parent}),
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

    fn nested_session(
        root: &Path,
        root_file: &str,
        id: &str,
        cwd: &Path,
        name: &str,
        message: &str,
    ) -> TestResult {
        let directory = root
            .join("custom/nested")
            .join(root_file)
            .join("agent/run-0");
        fs::create_dir_all(&directory)?;
        let lines = [
            serde_json::json!({"type":"session","version":3,"id":id,"timestamp":"2026-01-02T00:00:00Z","cwd":cwd}),
            serde_json::json!({"type":"message","message":{"role":"user","content":message}}),
            serde_json::json!({"type":"session_info","name":name}),
        ];
        fs::write(
            directory.join("session.jsonl"),
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

    #[test]
    fn discovery_sums_token_and_cost_usage_from_assistant_messages() -> TestResult {
        let root = tempdir()?;
        let project = tempdir()?;
        session(root.path(), "usage", project.path(), None, "Question")?;
        let path = root.path().join("custom/nested/usage.jsonl");
        let mut content = fs::read_to_string(&path)?;
        content.push_str(&format!(
            "\n{}",
            serde_json::json!({
                "type": "message",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Answer"}],
                    "usage": {
                        "input": 1000,
                        "output": 200,
                        "cacheRead": 3000,
                        "cacheWrite": 50,
                        "totalTokens": 4250,
                        "cost": {"total": 0.123456}
                    }
                }
            })
        ));
        fs::write(path, content)?;

        let sessions = discover_in(root.path(), project.path(), "")?;
        assert_eq!(
            sessions[0].usage,
            UsageSummary {
                input: 1000,
                output: 200,
                cache_read: 3000,
                cache_write: 50,
                total: 4250,
                cost_micros: 123_456,
            }
        );
        Ok(())
    }

    #[test]
    fn child_search_keeps_its_root_and_hierarchy_is_stable() -> TestResult {
        let root = tempdir()?;
        let project = tempdir()?;
        session(
            root.path(),
            "root",
            project.path(),
            Some("Main"),
            "ordinary",
        )?;
        nested_session(
            root.path(),
            "root",
            "child",
            project.path(),
            "subagent-reviewer-long-id",
            "Needle",
        )?;
        session_with_parent(
            root.path(),
            "grandchild",
            project.path(),
            Some("subagent-worker-long-id"),
            "Nested",
            Some("child"),
        )?;
        session_with_parent(
            root.path(),
            "orphan",
            project.path(),
            Some("subagent-worker-orphan-1"),
            "Detached",
            Some("missing"),
        )?;

        let sessions = discover_in(root.path(), project.path(), "needle")?;
        assert_eq!(sessions.len(), 3);
        let roots = root_sessions(&sessions);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, "root");
        let child = sessions
            .iter()
            .find(|session| session.id == "child")
            .expect("matching child should remain");
        assert_eq!(child.parent_session.as_deref(), Some("root"));
        assert_eq!(
            root_session_for_path(&sessions, Some(child.path.as_path()))
                .map(|session| session.id.as_str()),
            Some("root")
        );

        let all = discover_in(root.path(), project.path(), "")?;
        assert_eq!(
            root_sessions(&all)
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["root"]
        );
        let descendants = descendant_sessions(&all, "root");
        assert_eq!(
            descendants
                .iter()
                .map(|(session, depth)| (session.id.as_str(), *depth))
                .collect::<Vec<_>>(),
            vec![("child", 1), ("grandchild", 2)]
        );
        Ok(())
    }

    #[test]
    fn history_follows_the_current_branch_and_projects_display_entries() {
        let entries = vec![
            serde_json::json!({"type":"message","id":"one","parentId":null,"message":{"role":"user","content":"root"}}),
            serde_json::json!({"type":"message","id":"old","parentId":"one","message":{"role":"assistant","content":[{"type":"text","text":"old branch"}]}}),
            serde_json::json!({"type":"message","id":"two","parentId":"one","message":{"role":"assistant","content":[{"type":"text","text":"current"}]}}),
            serde_json::json!({"type":"custom_message","id":"three","parentId":"two","customType":"note","content":"visible","display":true}),
        ];

        let history = project_history(&entries);

        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["content"], "root");
        assert_eq!(history[1]["content"][0]["text"], "current");
        assert_eq!(history[2]["role"], "custom");
    }

    #[test]
    fn history_matches_pi_compaction_order() {
        let entries = vec![
            serde_json::json!({"type":"message","id":"one","parentId":null,"message":{"role":"user","content":"summarized"}}),
            serde_json::json!({"type":"message","id":"two","parentId":"one","message":{"role":"user","content":"kept"}}),
            serde_json::json!({"type":"compaction","id":"three","parentId":"two","summary":"summary","firstKeptEntryId":"two"}),
            serde_json::json!({"type":"message","id":"four","parentId":"three","message":{"role":"user","content":"after"}}),
        ];

        let history = project_history(&entries);

        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["role"], "compactionSummary");
        assert_eq!(history[1]["content"], "kept");
        assert_eq!(history[2]["content"], "after");
    }
}
