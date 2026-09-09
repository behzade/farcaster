//! Read-only native transcript discovery. Storage records are not CLI wire frames.
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use super::super::main_session::{external_session_locator, external_session_path};
use super::{
    BACKEND,
    events::{history_messages, string, text},
};
use crate::agents::{DiscoveredHistory, DiscoveredSession};
use serde_json::Value;

fn projects_root() -> Result<PathBuf, String> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
        .map(|root| root.join("projects"))
        .ok_or_else(|| "Claude config directory is unavailable".into())
}

fn files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read Claude projects: {error}")),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            continue;
        }
        for file in fs::read_dir(entry.path()).map_err(|error| error.to_string())? {
            let file = file.map_err(|error| error.to_string())?;
            let path = file.path();
            if file
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                && path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                files.push(path);
            }
        }
    }
    Ok(files)
}

// A child shares its parent's session UUID; include both IDs in its locator.
pub(super) fn child_id(parent: &str, agent: &str) -> Option<String> {
    uuid::Uuid::parse_str(parent).ok()?;
    if agent.is_empty()
        || !agent
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    Some(format!("{parent}/{agent}"))
}

fn child_files(parent: &Path, parent_id: &str) -> Result<Vec<(PathBuf, String)>, String> {
    let directory = parent.with_extension("").join("subagents");
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
            || path.extension().is_none_or(|ext| ext != "jsonl")
        {
            continue;
        }
        if let Some(id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix("agent-"))
            .and_then(|agent| child_id(parent_id, agent))
        {
            paths.push((path, id));
        }
    }
    Ok(paths)
}

fn read(path: &Path) -> Result<Vec<Value>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("read Claude transcript {}: {error}", path.display()))?;
    let mut rows = Vec::new();
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            break;
        }
        match serde_json::from_str(&line) {
            Ok(row) => rows.push(row),
            // An active append writer can leave its last record incomplete.
            Err(_) if !line.ends_with('\n') => break,
            Err(error) => {
                return Err(format!(
                    "invalid Claude transcript {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(rows)
}

// The SDK catalog samples the head and tail. Do not load every session's tool
// output just to draw the session list; only explicit history reads need it.
fn summary(path: &Path) -> Result<Vec<Value>, String> {
    const SAMPLE: u64 = 65_536;
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut head = Vec::new();
    (&mut file)
        .take(SAMPLE)
        .read_to_end(&mut head)
        .map_err(|error| error.to_string())?;
    let mut rows = head
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect::<Vec<_>>();
    if size > SAMPLE {
        let start = SAMPLE.max(size.saturating_sub(SAMPLE));
        file.seek(SeekFrom::Start(start - 1))
            .map_err(|error| error.to_string())?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)
            .map_err(|error| error.to_string())?;
        // The first line may start in the middle of an earlier record.
        rows.extend(
            tail.split(|byte| *byte == b'\n')
                .skip(1)
                .filter_map(|line| serde_json::from_slice::<Value>(line).ok()),
        );
    }
    Ok(rows)
}

/// Follow the newest main conversation's parent chain, not abandoned branches.
/// Compaction can reparent a preserved segment; apply those links as the SDK does.
fn conversation(rows: &[Value], sidechain: bool) -> Vec<&Value> {
    let mut nodes = HashMap::new();
    let mut parents = HashMap::new();
    for row in rows {
        if let Some(id) = row["uuid"].as_str() {
            nodes.insert(id, row);
            parents.insert(id, row["parentUuid"].as_str());
        }
    }
    for row in rows {
        let metadata = &row["compactMetadata"];
        let preserved = &metadata["preservedMessages"];
        let segment = &metadata["preservedSegment"];
        let (head, tail, anchor) = if let Some(ids) = preserved["uuids"].as_array() {
            let ids = ids.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            if ids.is_empty() || ids.iter().any(|id| !nodes.contains_key(id)) {
                continue;
            }
            let mut parent = preserved["anchorUuid"].as_str();
            for id in &ids {
                parents.insert(*id, parent);
                parent = Some(*id);
            }
            (
                ids[0],
                *ids.last().expect("preserved IDs checked nonempty above"),
                preserved["anchorUuid"].as_str(),
            )
        } else if let (Some(head), Some(tail)) =
            (segment["headUuid"].as_str(), segment["tailUuid"].as_str())
        {
            parents.insert(head, segment["anchorUuid"].as_str());
            (head, tail, segment["anchorUuid"].as_str())
        } else {
            continue;
        };
        for (id, parent) in &mut parents {
            if *parent == anchor && *id != head {
                *parent = Some(tail);
            }
        }
    }
    let latest = rows.iter().rev().find(|row| {
        matches!(string(row, "type"), "user" | "assistant")
            && (sidechain || row["isSidechain"] != true)
            && row["isMeta"] != true
            && row["teamName"].is_null()
    });
    let mut current = latest.and_then(|row| row["uuid"].as_str());
    let mut seen = HashSet::new();
    let mut chain = Vec::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            break;
        }
        let Some(row) = nodes.get(id) else {
            break;
        };
        chain.push(*row);
        current = parents.get(id).copied().flatten();
    }
    chain.reverse();
    chain
}

fn history(rows: &[Value], sidechain: bool) -> DiscoveredHistory {
    let chain = conversation(rows, sidechain);
    let model = chain
        .iter()
        .rev()
        .find_map(|row| row["message"]["model"].as_str())
        .map(|model| (BACKEND.into(), model.into()));
    DiscoveredHistory {
        messages: chain
            .iter()
            .filter(|row| row["isMeta"] != true)
            .flat_map(|row| history_messages(&row["message"]))
            .collect(),
        model,
        thinking_level: None,
    }
}

pub(in crate::modules::agents::adapter) fn load_history(
    path: &Path,
) -> Result<DiscoveredHistory, String> {
    load_history_in(&projects_root()?, path)
}

fn load_history_in(root: &Path, path: &Path) -> Result<DiscoveredHistory, String> {
    let id = external_session_locator(BACKEND, path).ok_or("invalid Claude session locator")?;
    let (parent, agent) = id
        .split_once('/')
        .map_or((id.as_str(), None), |(parent, agent)| (parent, Some(agent)));
    uuid::Uuid::parse_str(parent).map_err(|_| "invalid Claude session UUID")?;
    if let Some(agent) = agent {
        child_id(parent, agent).ok_or("invalid Claude agent ID")?;
    }
    let matching = files(root)?
        .into_iter()
        .filter(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(parent))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [path] => {
            let path = agent.map_or_else(
                || path.clone(),
                |agent| {
                    path.with_extension("")
                        .join("subagents")
                        .join(format!("agent-{agent}.jsonl"))
                },
            );
            read(&path).map(|rows| history(&rows, agent.is_some()))
        }
        [] => Err(format!("Claude transcript {id} not found")),
        _ => Err(format!(
            "Claude transcript {id} exists in more than one project"
        )),
    }
}

pub(in crate::modules::agents::adapter) fn discover(
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    discover_in(&projects_root()?, locator_root, query)
}

fn first_prompt(rows: &[Value], sidechain: bool) -> String {
    rows.iter()
        .filter(|row| {
            row["type"] == "user"
                && row["isMeta"] != true
                && (sidechain || row["isSidechain"] != true)
        })
        .map(|row| text(&row["message"]["content"]))
        .find(|text| !text.is_empty())
        .unwrap_or_default()
}

fn discover_in(
    root: &Path,
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let mut sessions = Vec::new();
    let query = query.to_lowercase();
    for path in files(root)? {
        let rows = match summary(&path) {
            Ok(rows) => rows,
            Err(error) => {
                zlog::warn!("{error}");
                continue;
            }
        };
        let Some(project) = rows
            .iter()
            .rev()
            .find_map(|row| row["relocatedCwd"].as_str().or_else(|| row["cwd"].as_str()))
        else {
            continue;
        };
        let project = PathBuf::from(project);
        if !project.is_absolute()
            || !project.is_dir()
            || crate::projects::is_temporary_project(&project)
        {
            continue;
        }
        if rows.first().is_some_and(|row| row["isSidechain"] == true) {
            continue;
        }
        let first = first_prompt(&rows, false);
        let title = rows
            .iter()
            .rev()
            .find_map(|row| {
                row["customTitle"]
                    .as_str()
                    .or_else(|| row["aiTitle"].as_str())
            })
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if first.is_empty() {
                    "Claude Code".into()
                } else {
                    first.chars().take(120).collect()
                }
            });
        let search = format!("{title} {first} {} Claude Code", project.display());
        let id = path
            .file_stem()
            .expect("session path has a JSONL filename")
            .to_string_lossy()
            .into_owned();
        let parent = DiscoveredSession {
            path: external_session_path(locator_root, BACKEND, &id),
            id,
            harness: BACKEND.into(),
            project,
            title,
            first_user_message: first,
            timestamp: rows
                .iter()
                .rev()
                .find_map(|row| row["timestamp"].as_str())
                .unwrap_or_default()
                .into(),
            modified: fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH),
            message_count: 0,
            model: rows
                .iter()
                .rev()
                .find_map(|row| row["message"]["model"].as_str())
                .map(|model| (BACKEND.into(), model.into())),
            thinking_level: None,
            usage: Default::default(),
            parent_session: None,
            archived: false,
            is_running: false,
            search,
        };
        for (child_path, id) in child_files(&path, &parent.id)? {
            let rows = match summary(&child_path) {
                Ok(rows) => rows,
                Err(_) => continue,
            };
            let metadata: Value = fs::read(child_path.with_extension("meta.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let first = first_prompt(&rows, true);
            let title = metadata["description"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| first.chars().take(120).collect());
            let search = format!("{title} {first} {} Claude Code", parent.project.display());
            sessions.push(DiscoveredSession {
                path: external_session_path(locator_root, BACKEND, &id),
                id,
                title,
                first_user_message: first,
                search,
                parent_session: Some(parent.id.clone()),
                timestamp: rows
                    .iter()
                    .rev()
                    .find_map(|row| row["timestamp"].as_str())
                    .unwrap_or_default()
                    .into(),
                model: rows
                    .iter()
                    .rev()
                    .find_map(|row| row["message"]["model"].as_str())
                    .map(|model| (BACKEND.into(), model.into())),
                modified: fs::metadata(&child_path)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::UNIX_EPOCH),
                ..parent.clone()
            });
        }
        sessions.push(parent);
    }
    sessions.retain(|session| session.search.to_lowercase().contains(&query));
    sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
    Ok(sessions)
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
