use std::{
    cell::Cell,
    io::{ErrorKind, Read as _, Seek as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use super::super::super::history_cache::{FileStamp, HistoryCache};
use crate::DiscoveredHistory;

type Revision = (FileStamp, Option<FileStamp>);
type Key = (PathBuf, PathBuf, String);
static HISTORY: HistoryCache<Key, Revision, DiscoveredHistory> = HistoryCache::new();

pub(super) fn load(
    locator: &str,
    load: impl FnOnce() -> Result<DiscoveredHistory, String>,
) -> Result<DiscoveredHistory, String> {
    let program = super::super::program();
    // Ask the same executable used by the catalog server. Guessing XDG paths
    // misses channel databases and OPENCODE_DB overrides (including :memory:).
    let Some(database) = database_path(&program) else {
        return load();
    };
    load_database(&HISTORY, (program, database, locator.to_owned()), load)
}

fn load_database(
    cache: &HistoryCache<Key, Revision, DiscoveredHistory>,
    key: Key,
    load: impl FnOnce() -> Result<DiscoveredHistory, String>,
) -> Result<DiscoveredHistory, String> {
    let database = key.1.clone();
    let complete = Cell::new(true);
    cache.load(
        key,
        || complete.get().then(|| revision(&database)).flatten(),
        || {
            let history = load()?;
            // An inbox error is transient, not an authoritative empty inbox.
            complete.set(history.prompt_deliveries.is_some());
            Ok(history)
        },
    )
}

fn database_path(program: &Path) -> Option<PathBuf> {
    let mut command = Command::new(program);
    command.args(["debug", "paths"]);
    let output = discover_paths(&mut command, Duration::from_secs(2))?;
    // SQLite resolves database symlinks before locating its WAL sidecar.
    parse_database_path(std::str::from_utf8(&output).ok()?)?
        .canonicalize()
        .ok()
}

fn discover_paths(command: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    // Discovery is only an optimization. An unsupported or stuck command must
    // fall back to normal history loading, not leave the selection hanging.
    // A file avoids a full stdout pipe or inherited pipe handles blocking exit.
    let mut output = tempfile::tempfile().ok()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(output.try_clone().ok()?)
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    if output.metadata().ok()?.len() > 64 * 1024 {
        return None;
    }
    output.rewind().ok()?;
    let mut bytes = Vec::new();
    output.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn parse_database_path(output: &str) -> Option<PathBuf> {
    let mut paths = output.lines().filter_map(|line| {
        let rest = line.strip_prefix("db")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        Some(PathBuf::from(rest.trim()))
    });
    let path = paths.next()?;
    (path.is_absolute() && paths.next().is_none()).then_some(path)
}

fn revision(database: &Path) -> Option<Revision> {
    // session_v2 (model), session_message and session_inbox share this SQLite
    // database. WAL writes can change any of them without touching the main
    // file. A rollback journal means a transaction/recovery may be underway.
    let journal = sidecar(database, "-journal");
    match std::fs::symlink_metadata(journal) {
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        _ => return None,
    }
    let main = FileStamp::read(database)?;
    let wal_path = sidecar(database, "-wal");
    let wal = match std::fs::symlink_metadata(&wal_path) {
        Ok(_) => Some(FileStamp::read(&wal_path)?),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(_) => return None,
    };
    Some((main, wal))
}

fn sidecar(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
