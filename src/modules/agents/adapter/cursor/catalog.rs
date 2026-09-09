use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use super::PROFILE;
use crate::agents::{DiscoveredSession, DiscoveredUsage};

#[derive(Deserialize, Serialize)]
struct SessionMeta {
    cwd: PathBuf,
    #[serde(default)]
    title: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

fn session_root() -> Result<PathBuf, String> {
    let env = |key| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    config_root(
        env("CURSOR_CONFIG_DIR"),
        env("XDG_CONFIG_HOME"),
        env("HOME"),
    )
    .map(|root| root.join("acp-sessions"))
}

fn config_root(
    explicit: Option<String>,
    xdg: Option<String>,
    home: Option<String>,
) -> Result<PathBuf, String> {
    if let Some(root) = explicit {
        return Ok(root.into());
    }
    if let Some(root) = xdg {
        return Ok(PathBuf::from(root).join("cursor"));
    }
    home.map(|root| PathBuf::from(root).join(".cursor"))
        .ok_or_else(|| "HOME is required to inspect Cursor sessions".into())
}

fn find_session_at(root: &Path, session_id: &str) -> Result<PathBuf, String> {
    if session_id.is_empty()
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Cursor session id is not safe to modify".into());
    }
    let path = root.join(session_id);
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("Cursor session was not found: {session_id}: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing unsafe Cursor session path: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn find_session(session_id: &str) -> Result<PathBuf, String> {
    find_session_at(&session_root()?, session_id)
}

fn metadata(directory: &Path) -> Result<SessionMeta, String> {
    let path = directory.join("meta.json");
    let bytes =
        std::fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let meta: SessionMeta = serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode {}: {error}", path.display()))?;
    if !meta.cwd.is_absolute() {
        return Err(format!(
            "Cursor session cwd is not absolute: {}",
            path.display()
        ));
    }
    Ok(meta)
}

fn session_data(directory: &Path) -> Result<(SessionMeta, bool), String> {
    let meta = metadata(directory)?;
    let unpersisted = match std::fs::symlink_metadata(directory.join("store.db")) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(false),
        Ok(_) => Err("unsafe Cursor session database".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(format!("inspect Cursor session database: {error}")),
    }?;
    Ok((meta, unpersisted))
}

pub(super) fn inspect(session_id: &str) -> Result<(PathBuf, bool), String> {
    let (meta, unpersisted) = session_data(&find_session(session_id)?)?;
    Ok((meta.cwd, unpersisted))
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
    rename_at(&find_session(session_id)?, name)
}

fn rename_at(directory: &Path, name: &str) -> Result<(), String> {
    let (mut meta, draft) = session_data(directory)?;
    if !draft {
        let connection = Connection::open_with_flags(
            directory.join("store.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|error| format!("open Cursor session: {error}"))?;
        let encoded: String = connection
            .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
                row.get(0)
            })
            .map_err(|error| format!("read Cursor session metadata: {error}"))?;
        let mut stored: serde_json::Value = serde_json::from_slice(
            &decode_hex(&encoded).ok_or_else(|| "Cursor session metadata is not hex".to_owned())?,
        )
        .map_err(|error| format!("decode Cursor session metadata: {error}"))?;
        stored["name"] = name.into();
        connection
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = '0'",
                [encode_hex(
                    &serde_json::to_vec(&stored).map_err(|error| error.to_string())?,
                )],
            )
            .map_err(|error| format!("rename Cursor session: {error}"))?;
    }
    meta.title = Some(name.into());
    std::fs::write(
        directory.join("meta.json"),
        serde_json::to_vec(&meta).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("rename Cursor sidecar: {error}"))
}

pub(super) fn discover(locator_root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
    let query = query.to_ascii_lowercase();
    Ok(super::super::acp::list_sessions(&PROFILE)?
        .iter()
        .filter_map(|session| listed_session(locator_root, &query, session))
        .collect())
}

fn listed_session(
    locator_root: &Path,
    query: &str,
    value: &serde_json::Value,
) -> Option<DiscoveredSession> {
    let id = value.get("sessionId")?.as_str()?.to_owned();
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return None;
    }
    let project = PathBuf::from(value.get("cwd")?.as_str()?);
    if !project.is_absolute()
        || !project.is_dir()
        || crate::projects::is_temporary_project(&project)
    {
        return None;
    }
    let title = value
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("New Cursor session")
        .to_owned();
    let search = format!("{title} {} {}", project.display(), PROFILE.name);
    if !query.is_empty() && !search.to_ascii_lowercase().contains(query) {
        return None;
    }
    let timestamp = value
        .get("updatedAt")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let modified =
        time::OffsetDateTime::parse(&timestamp, &time::format_description::well_known::Rfc3339)
            .ok()
            .map(std::time::SystemTime::from)
            .unwrap_or(UNIX_EPOCH);
    Some(DiscoveredSession {
        path: super::super::main_session::external_session_path(locator_root, PROFILE.backend, &id),
        parent_session: crate::modules::agents::core::CallerRegistry::shared()
            .session_parent(PROFILE.backend, &id),
        id,
        harness: PROFILE.backend.into(),
        project,
        title,
        first_user_message: String::new(),
        timestamp,
        modified,
        message_count: 0,
        usage: DiscoveredUsage::default(),
        archived: false,
        is_running: false,
        model: None,
        thinking_level: None,
        search,
    })
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

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
