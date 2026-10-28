use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::super::main_session::external_session_locator;
use crate::Backend;

/// Verify ancestry without starting an app-server during storage migration.
pub(super) fn read(path: &Path, data_directory: Option<&Path>) -> Option<String> {
    let id = external_session_locator(Backend::Codex, path)?;
    // A custom profile must never read a same-ID session from the default home.
    if crate::profile_id_from_locator(path).is_some() && data_directory.is_none() {
        return None;
    }
    let home = data_directory.map(Path::to_path_buf).or_else(|| {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
    })?;
    let connection = Connection::open_with_flags(
        home.join("state_5.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    connection.busy_timeout(std::time::Duration::ZERO).ok()?;
    let rollout: String = connection
        .query_row(
            "SELECT rollout_path FROM threads WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .ok()?;
    let mut header = String::new();
    BufReader::new(File::open(rollout).ok()?)
        .read_line(&mut header)
        .ok()?;
    let header: Value = serde_json::from_str(&header).ok()?;
    let payload = &header["payload"];
    if header["type"] != "session_meta"
        || payload["id"].as_str() != Some(&id)
        || !super::catalog::ordinary_source(&payload["source"])
        || crate::core::CallerRegistry::shared()
            .session_parent(Backend::Codex, &id)
            .is_some()
    {
        return None;
    }
    let source = payload["forked_from_id"]
        .as_str()
        .or_else(|| payload["parent_thread_id"].as_str())?;
    (!source.is_empty() && source != id).then(|| source.to_owned())
}

#[cfg(test)]
#[path = "fork_source_tests.rs"]
mod tests;
