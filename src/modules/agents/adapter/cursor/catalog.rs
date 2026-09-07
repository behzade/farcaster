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

// Match cursor-config/paths: an explicit override wins over XDG, then HOME.
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

// A sidecar is allocated by session/new before Cursor has persisted any turns.
// Only this known draft state may be restarted; missing/corrupt sessions are errors.
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
    discover_at(&session_root()?, locator_root, query)
}

fn discover_at(
    root: &Path,
    locator_root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read Cursor sessions: {error}")),
    };
    let query = query.to_ascii_lowercase();
    Ok(entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().into_string().ok()?;
            let directory = find_session_at(root, &id).ok()?;
            read_session(locator_root, &query, &directory)
        })
        .collect())
}

fn read_session(locator_root: &Path, query: &str, directory: &Path) -> Option<DiscoveredSession> {
    let (meta, unpersisted) = session_data(directory).ok()?;
    if unpersisted {
        return None;
    }
    let project = meta.cwd;
    if !project.is_dir() || crate::projects::is_temporary_project(&project) {
        return None;
    }
    let id = directory.file_name()?.to_str()?.to_owned();
    let title = meta
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "New Cursor session".into());
    let search = format!("{title} {} {}", project.display(), PROFILE.name);
    if !query.is_empty() && !search.to_ascii_lowercase().contains(query) {
        return None;
    }
    let modified = ["store.db", "store.db-wal", "meta.json"]
        .into_iter()
        .filter_map(|name| directory.join(name).metadata().ok()?.modified().ok())
        .max()
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
        timestamp: String::new(),
        modified,
        // ACP root blobs can be encrypted; history is replayed by Cursor, not decoded here.
        message_count: 0,
        usage: DiscoveredUsage::default(),
        archived: false,
        is_running: false,
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
mod tests {
    use super::*;

    fn fixture(root: &Path, id: &str, persisted: bool) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::fs::write(
            dir.join("meta.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1, "cwd": cwd, "title": "ACP fixture", "futureField": true
            }))
            .unwrap(),
        )
        .unwrap();
        if persisted {
            let db = Connection::open(dir.join("store.db")).unwrap();
            db.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);")
                .unwrap();
            db.execute(
                "INSERT INTO meta VALUES ('0', ?1)",
                [encode_hex(
                    br#"{"name":"ACP fixture","blobEncryptionKey":"preserve-me"}"#,
                )],
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn config_directory_matches_cursor_precedence() {
        assert_eq!(
            config_root(Some("/override".into()), Some("/xdg".into()), None).unwrap(),
            PathBuf::from("/override")
        );
        assert_eq!(
            config_root(None, Some("/xdg".into()), None).unwrap(),
            PathBuf::from("/xdg/cursor")
        );
        assert_eq!(
            config_root(None, None, Some("/home/user".into())).unwrap(),
            PathBuf::from("/home/user/.cursor")
        );
        assert!(config_root(None, None, None).is_err());
    }

    #[test]
    fn acp_catalog_excludes_unpersisted_drafts_and_uses_sidecar() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), "persisted", true);
        let draft = fixture(root.path(), "draft", false);
        assert!(session_data(&draft).unwrap().1);
        let sessions = discover_at(root.path(), root.path(), "ACP fixture").unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "persisted");
        assert_eq!(sessions[0].project, std::env::current_dir().unwrap());
        assert!(
            discover_at(root.path(), root.path(), "does-not-match")
                .unwrap()
                .is_empty()
        );
        assert!(find_session_at(root.path(), "missing").is_err());
        assert!(find_session_at(root.path(), "../escape").is_err());
        std::fs::write(draft.join("meta.json"), b"broken").unwrap();
        assert!(session_data(&draft).is_err());
    }

    #[test]
    fn rename_updates_database_and_sidecar_without_losing_fields() {
        let root = tempfile::tempdir().unwrap();
        let dir = fixture(root.path(), "persisted", true);
        rename_at(&dir, "Renamed").unwrap();
        let meta = metadata(&dir).unwrap();
        assert_eq!(meta.title.as_deref(), Some("Renamed"));
        assert_eq!(meta.extra["futureField"], true);
        let db = Connection::open(dir.join("store.db")).unwrap();
        let encoded: String = db
            .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let stored: serde_json::Value =
            serde_json::from_slice(&decode_hex(&encoded).unwrap()).unwrap();
        assert_eq!(stored["name"], "Renamed");
        assert_eq!(stored["blobEncryptionKey"], "preserve-me");
        let draft = fixture(root.path(), "draft", false);
        rename_at(&draft, "Draft title").unwrap();
        assert!(session_data(&draft).unwrap().1);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_sessions_and_databases() {
        let root = tempfile::tempdir().unwrap();
        let dir = fixture(root.path(), "real", true);
        std::os::unix::fs::symlink(&dir, root.path().join("link")).unwrap();
        assert!(find_session_at(root.path(), "link").is_err());
        let draft = fixture(root.path(), "draft", false);
        std::os::unix::fs::symlink(dir.join("store.db"), draft.join("store.db")).unwrap();
        assert!(session_data(&draft).is_err());
    }
}
