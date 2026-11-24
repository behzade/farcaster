use std::{
    ffi::{OsStr, OsString},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
};

use rusqlite::{Connection, OpenFlags};

use super::super::super::history_cache::{FileStamp, HistoryCache};
use crate::DiscoveredHistory;

#[derive(Clone, Eq, Hash, PartialEq)]
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
    fn effective_env(&self, key: &str) -> Option<&OsStr> {
        if self.profile.is_some() {
            self.env.iter().find(|(k, _)| k == key)?.1.as_deref()
        } else {
            self.inherited_env
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_os_str())
        }
    }

    pub(super) fn local_revision(
        &self,
        thread: &str,
        reported_home: Option<&Path>,
    ) -> Option<serde_json::Value> {
        if self.has_sqlite_home_override() {
            return None;
        }
        let home = self
            .effective_env("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                self.effective_env("HOME")
                    .map(|p| PathBuf::from(p).join(".codex"))
            })?;
        if !home.is_absolute() {
            return None;
        }
        let home = home.canonicalize().ok()?;
        if let Some(reported) = reported_home
            && reported.canonicalize().ok()? != home
        {
            return None;
        }
        if !known_database_versions(&home)? {
            return None;
        }
        let revision = revision(&home, thread)?;
        let cwd = self.cwd.as_deref()?;
        let executable = super::super::super::process_command::resolve_agent_program(
            Path::new(&self.program),
            cwd,
            self.effective_env("PATH"),
        )
        .ok()?;
        let executable_stamp = FileStamp::read(&executable)?;
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        let config_paths = [
            home.join("config.toml"),
            PathBuf::from("/etc/codex/config.toml"),
            PathBuf::from("/etc/codex/managed_config.toml"),
        ]
        .into_iter()
        .chain(cwd.ancestors().map(|dir| dir.join(".codex/config.toml")));
        let config_files = config_paths
            .map(|path| {
                let stamp = optional_file_stamp(&path)?;
                Some((path, stamp))
            })
            .collect::<Option<Vec<_>>>()?;
        serde_json::to_value((
            "codex-v1",
            home,
            thread,
            revision,
            executable,
            executable_stamp,
            hasher.finish(),
            config_files,
        ))
        .ok()
    }

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
#[derive(Eq, PartialEq, serde::Serialize)]
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

fn state_database(home: &Path) -> Option<Connection> {
    if !home.is_absolute() {
        return None;
    }
    let db = Connection::open_with_flags(
        home.join("state_5.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    db.busy_timeout(std::time::Duration::ZERO).ok()?;
    Some(db)
}

fn known_database_versions(home: &Path) -> Option<bool> {
    for entry in std::fs::read_dir(home).ok()? {
        let name = entry.ok()?.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".sqlite")) else {
            continue;
        };
        for (prefix, supported) in [("state_", 5), ("thread_history_", 1)] {
            if stem
                .strip_prefix(prefix)
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|v| v > supported)
            {
                return Some(false);
            }
        }
    }
    Some(true)
}

// The full-history cache tracks all WAL writes. For a provisional page, use
// Codex's per-thread cursor so activity in another thread cannot hide the page.
pub(super) fn paginated_revision(home: &Path, thread: &str) -> Option<impl Eq> {
    if !known_database_versions(home)? {
        return None;
    }
    let rollout: String = state_database(home)?
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1 AND history_mode = 'paginated'",
            [thread],
            |row| row.get(0),
        )
        .ok()?;
    let rollout = PathBuf::from(rollout);
    if !rollout.is_absolute() {
        return None;
    }
    let rollout = rollout.canonicalize().ok()?;
    let metadata = rollout.metadata().ok()?;
    let database = home.join("thread_history_1.sqlite").canonicalize().ok()?;
    let db = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    db.busy_timeout(std::time::Duration::ZERO).ok()?;
    let (offset, ordinal): (u64, u64) = db
        .query_row(
            "SELECT next_rollout_byte_offset, next_rollout_ordinal
         FROM thread_history_projection_state WHERE thread_id = ?1
         AND EXISTS (SELECT 1 FROM thread_turns WHERE thread_id = ?1)",
            [thread],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    if !metadata.is_file() || offset != metadata.len() {
        return None;
    }
    Some((
        rollout,
        metadata.modified().ok()?,
        database,
        offset,
        ordinal,
    ))
}

fn revision(home: &Path, thread: &str) -> Option<Revision> {
    let connection = state_database(home)?;
    let (rollout, mode, identity): (String, String, Identity) = connection
        .query_row(
            "SELECT rollout_path, history_mode, model_provider, model, reasoning_effort FROM threads WHERE id = ?1",
            [thread],
            |row| Ok((row.get(0)?, row.get(1)?, Identity { provider: row.get(2)?, model: row.get(3)?, effort: row.get(4)? })),
        )
        .ok()?;
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

fn optional_file_stamp(path: &Path) -> Option<Option<FileStamp>> {
    match std::fs::metadata(path) {
        Ok(_) => Some(Some(FileStamp::read(path)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

fn database_stamps(home: &Path) -> Option<DatabaseStamps> {
    let mut stamps = Vec::new();
    for database in ["thread_history_1.sqlite"] {
        let reported = home.join(database);
        let database = match std::fs::symlink_metadata(&reported) {
            Ok(_) => std::fs::canonicalize(&reported).ok()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => reported,
            Err(_) => return None,
        };
        for suffix in ["", "-wal", "-journal"] {
            let mut path = database.as_os_str().to_owned();
            path.push(suffix);
            let path = PathBuf::from(path);
            let stamp = optional_file_stamp(&path)?;
            stamps.push((path, stamp));
        }
    }
    stamps[0].1.as_ref()?;
    Some(stamps)
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
