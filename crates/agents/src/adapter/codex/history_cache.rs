use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use rusqlite::{Connection, OpenFlags};

use super::super::super::history_cache::{FileStamp, HistoryCache};
use crate::DiscoveredHistory;

#[derive(Eq, PartialEq)]
pub(super) struct Scope {
    program: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, Option<OsString>)>,
    inherited_env: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
    project: Option<PathBuf>,
    profile: Option<String>,
}

impl Scope {
    pub(super) fn has_sqlite_home_override(&self) -> bool {
        self.env
            .iter()
            .any(|(key, value)| key == "CODEX_SQLITE_HOME" && value.is_some())
            || self
                .inherited_env
                .iter()
                .any(|(key, _)| key == "CODEX_SQLITE_HOME")
    }
    pub(super) fn new(
        command: &Command,
        config: Option<&crate::AgentLaunchConfig>,
        project: Option<&Path>,
    ) -> Self {
        // Command exposes explicit overrides, but default launches also inherit
        // PATH and provider/configuration variables from the process.
        let mut inherited_env: Vec<_> = std::env::vars_os().collect();
        inherited_env.sort();
        Self {
            program: command.get_program().to_owned(),
            args: command.get_args().map(OsString::from).collect(),
            env: command
                .get_envs()
                .map(|(key, value)| (key.to_owned(), value.map(OsString::from)))
                .collect(),
            inherited_env,
            cwd: command
                .get_current_dir()
                .map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok()),
            project: project.map(Path::to_path_buf),
            profile: config.and_then(|config| config.profile_id.clone()),
        }
    }
}

type DatabaseStamps = Vec<(PathBuf, Option<FileStamp>)>;
#[derive(Eq, PartialEq)]
struct Identity {
    provider: String,
    model: Option<String>,
    effort: Option<String>,
}
type Revision = (PathBuf, FileStamp, Identity, DatabaseStamps);
type Key = (PathBuf, String, Scope);
static CACHE: HistoryCache<Key, Revision, DiscoveredHistory> = HistoryCache::new();

pub(super) fn load(
    home: &Path,
    thread: &str,
    scope: Scope,
    read: impl FnOnce() -> Result<DiscoveredHistory, String>,
) -> Result<DiscoveredHistory, String> {
    CACHE.load(
        (home.to_owned(), thread.to_owned(), scope),
        || revision(home, thread),
        read,
    )
}

// thread/read hydrates persisted history. Its inputs include the native rollout,
// state identity, and (for paginated histories) the separate history database.
// Read identity through SQLite: app-server startup can change global state file
// stamps even when this thread did not change. History DB stamps include WALs.
fn revision(home: &Path, thread: &str) -> Option<Revision> {
    if !home.is_absolute() {
        return None;
    }
    let connection = Connection::open_with_flags(
        home.join("state_5.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let (rollout, mode, identity): (String, String, Identity) = connection
        .query_row(
            "SELECT rollout_path, history_mode, model_provider, model, reasoning_effort FROM threads WHERE id = ?1",
            [thread],
            |row| Ok((row.get(0)?, row.get(1)?, Identity { provider: row.get(2)?, model: row.get(3)?, effort: row.get(4)? })),
        )
        .ok()?;
    // Unknown schemas/modes and missing files use the existing app-server path.
    if mode != "legacy" && mode != "paginated" {
        return None;
    }
    let databases = if mode == "paginated" {
        database_stamps(home)?
    } else {
        Vec::new()
    };
    let rollout = PathBuf::from(rollout);
    let stamp = FileStamp::read(&rollout)?;
    Some((rollout, stamp, identity, databases))
}

pub(super) fn has_revision(home: &Path, thread: &str) -> bool {
    revision(home, thread).is_some()
}

fn database_stamps(home: &Path) -> Option<DatabaseStamps> {
    let mut stamps = Vec::new();
    for database in ["thread_history_1.sqlite"] {
        let reported = home.join(database);
        // SQLite resolves a database symlink before locating its WAL/journal.
        // Track the resolved path too, so retargeting a link invalidates entries.
        let database = match std::fs::symlink_metadata(&reported) {
            Ok(_) => std::fs::canonicalize(&reported).ok()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => reported,
            Err(_) => return None,
        };
        for suffix in ["", "-wal", "-journal"] {
            let mut path = database.as_os_str().to_owned();
            path.push(suffix);
            let path = PathBuf::from(path);
            let stamp = match std::fs::metadata(&path) {
                Ok(_) => Some(FileStamp::read(&path)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return None,
            };
            stamps.push((path, stamp));
        }
    }
    stamps[0].1.as_ref()?;
    Some(stamps)
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
