use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::agents::{DiscoveredSession, DiscoveredUsage};

use super::PROFILE;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorMeta {
    agent_id: String,
    #[serde(default)]
    latest_root_blob_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    created_at: i64,
}

pub(super) fn discover(locator_root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
    let query = query.to_ascii_lowercase();
    let mut sessions = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for workspace in workspace_directories()? {
        let Ok(chats) = std::fs::read_dir(workspace) else {
            continue;
        };
        for chat in chats.flatten() {
            if !chat.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            if let Some(session) = read_session(locator_root, &query, &chat.path())
                && seen.insert(session.id.clone())
            {
                sessions.push(session);
            }
        }
    }
    Ok(sessions)
}

pub(super) fn project(session_id: &str) -> Result<PathBuf, String> {
    let directory = find_session(session_id)?;
    session_data(&directory)
        .map(|(_, project, _)| project)
        .ok_or_else(|| format!("could not read Cursor session: {}", directory.display()))
}

pub(in crate::modules::agents::adapter) fn delete(session_id: &str) -> Result<(), String> {
    let directory = find_session(session_id)?;
    std::fs::remove_dir_all(&directory)
        .map_err(|error| format!("delete Cursor session {}: {error}", directory.display()))
}

pub(in crate::modules::agents::adapter) fn rename(
    session_id: &str,
    name: &str,
) -> Result<(), String> {
    let database = find_session(session_id)?.join("store.db");
    if !database.is_file() {
        return Err(format!(
            "Cursor session database was not found: {}",
            database.display()
        ));
    }
    let connection = Connection::open(&database)
        .map_err(|error| format!("open Cursor session {}: {error}", database.display()))?;
    let encoded: String = connection
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            row.get(0)
        })
        .map_err(|error| format!("read Cursor session metadata: {error}"))?;
    let mut metadata: serde_json::Value = serde_json::from_slice(
        &decode_hex(&encoded).ok_or_else(|| "Cursor session metadata is not hex".to_owned())?,
    )
    .map_err(|error| format!("decode Cursor session metadata: {error}"))?;
    metadata["name"] = serde_json::Value::String(name.to_owned());
    let bytes = serde_json::to_vec(&metadata)
        .map_err(|error| format!("encode Cursor session metadata: {error}"))?;
    connection
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = '0'",
            [encode_hex(&bytes)],
        )
        .map_err(|error| format!("rename Cursor session: {error}"))?;
    Ok(())
}

fn find_session(session_id: &str) -> Result<PathBuf, String> {
    if session_id.is_empty()
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Cursor session id is not safe to modify".into());
    }
    for workspace in workspace_directories()? {
        let candidate = workspace.join(session_id);
        let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "refusing to modify unsafe Cursor session path: {}",
                candidate.display()
            ));
        }
        return Ok(candidate);
    }
    Err(format!("Cursor session was not found: {session_id}"))
}

fn workspace_directories() -> Result<Vec<PathBuf>, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is required to inspect Cursor sessions".to_owned())?;
    let roots = [
        home.join(".cursor/chats"),
        home.join(".config/cursor/chats"),
    ];
    Ok(roots
        .into_iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect())
}

fn session_data(directory: &Path) -> Option<(CursorMeta, PathBuf, usize)> {
    let connection =
        Connection::open_with_flags(directory.join("store.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    let encoded: String = connection
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            row.get(0)
        })
        .ok()?;
    let meta: CursorMeta = serde_json::from_slice(&decode_hex(&encoded)?).ok()?;
    let root = connection
        .query_row(
            "SELECT data FROM blobs WHERE id = ?1",
            [&meta.latest_root_blob_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .unwrap_or_default();
    let (project, message_count) = decode_root(&root)?;
    Some((meta, project, message_count))
}

fn read_session(locator_root: &Path, query: &str, directory: &Path) -> Option<DiscoveredSession> {
    let database = directory.join("store.db");
    let (meta, project, message_count) = session_data(directory)?;
    if !project.is_dir() || crate::projects::is_temporary_project(&project) {
        return None;
    }
    let title = if meta.name.trim().is_empty() {
        "New Cursor session".into()
    } else {
        meta.name
    };
    let search = format!("{title} {} {}", project.display(), PROFILE.name);
    if !query.is_empty() && !search.to_ascii_lowercase().contains(query) {
        return None;
    }
    let modified = [database.clone(), database.with_file_name("store.db-wal")]
        .into_iter()
        .filter_map(|path| path.metadata().ok()?.modified().ok())
        .max()
        .unwrap_or_else(|| timestamp(meta.created_at));
    Some(DiscoveredSession {
        id: meta.agent_id.clone(),
        harness: PROFILE.backend.into(),
        path: super::super::main_session::external_session_path(
            locator_root,
            PROFILE.backend,
            &meta.agent_id,
        ),
        project,
        title,
        first_user_message: String::new(),
        timestamp: String::new(),
        parent_session: crate::modules::agents::core::CallerRegistry::shared()
            .session_parent(PROFILE.backend, &meta.agent_id),
        modified,
        message_count,
        usage: DiscoveredUsage::default(),
        archived: false,
        is_running: false,
        search,
    })
}

fn timestamp(milliseconds: i64) -> SystemTime {
    u64::try_from(milliseconds)
        .ok()
        .map(|milliseconds| UNIX_EPOCH + Duration::from_millis(milliseconds))
        .unwrap_or(UNIX_EPOCH)
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks_exact(2)
        .map(|pair| Some((hex(pair[0])? << 4) | hex(pair[1])?))
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_root(bytes: &[u8]) -> Option<(PathBuf, usize)> {
    let mut input = bytes;
    let mut workspace = None;
    let mut messages = 0;
    while !input.is_empty() {
        let key = take_varint(&mut input)?;
        let field = key >> 3;
        let wire = key & 7;
        match wire {
            0 => {
                take_varint(&mut input)?;
            }
            1 => input = input.get(8..)?,
            2 => {
                let length = usize::try_from(take_varint(&mut input)?).ok()?;
                let value = input.get(..length)?;
                input = input.get(length..)?;
                if field == 1 {
                    messages += 1;
                } else if field == 9 {
                    workspace = std::str::from_utf8(value)
                        .ok()
                        .and_then(|value| url::Url::parse(value).ok())
                        .and_then(|url| url.to_file_path().ok());
                }
            }
            5 => input = input.get(4..)?,
            _ => return None,
        }
    }
    workspace.map(|workspace| (workspace, messages))
}

fn take_varint(input: &mut &[u8]) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let byte = *input.first()?;
        *input = input.get(1..)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_cursor_root_workspace_and_message_count() {
        let workspace = b"file:///tmp/project";
        let mut root = vec![0x0a, 1, b'a', 0x0a, 1, b'b', 0x4a, workspace.len() as u8];
        root.extend(workspace);
        assert_eq!(decode_root(&root), Some((PathBuf::from("/tmp/project"), 2)));
    }
}
