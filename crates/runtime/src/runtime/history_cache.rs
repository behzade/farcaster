use super::*;
use std::sync::{Mutex, MutexGuard};

const LIMIT: usize = 24;

#[derive(Clone, Eq, PartialEq)]
struct Stamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

fn stamp(path: &std::path::Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

struct Entry {
    stamp: Option<Stamp>,
    history: LoadedHistory,
}

struct HistoryCache {
    entries: Vec<(PathBuf, Entry)>,
}

impl HistoryCache {
    fn take(&mut self, path: &std::path::Path, stamp: Option<&Stamp>) -> Option<LoadedHistory> {
        let (index, history) =
            self.entries
                .iter()
                .enumerate()
                .find_map(|(index, (key, entry))| {
                    (key.as_path() == path
                        && entry.stamp.is_some()
                        && entry.stamp.as_ref() == stamp)
                        .then(|| (index, entry.history.clone()))
                })?;
        let entry = self.entries.remove(index);
        self.entries.push(entry);
        Some(history)
    }

    fn contains(&self, path: &std::path::Path, stamp: Option<&Stamp>) -> bool {
        self.entries.iter().any(|(key, entry)| {
            key.as_path() == path && entry.stamp.is_some() && entry.stamp.as_ref() == stamp
        })
    }

    fn store(&mut self, path: PathBuf, stamp: Option<Stamp>, history: &LoadedHistory) {
        self.entries.retain(|(key, _)| key != &path);
        if stamp.is_none() {
            return;
        }
        self.entries.push((
            path,
            Entry {
                stamp,
                history: history.clone(),
            },
        ));
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
    }
}

static CACHE: Mutex<HistoryCache> = Mutex::new(HistoryCache {
    entries: Vec::new(),
});

fn cache() -> MutexGuard<'static, HistoryCache> {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn history_is_fresh(path: &std::path::Path) -> bool {
    let Some(stamp) = stamp(path) else {
        return false;
    };
    cache().contains(path, Some(&stamp))
}

pub fn load_cached_history(
    harness: Backend,
    path: &std::path::Path,
    project: &std::path::Path,
) -> Result<LoadedHistory, String> {
    let stamp = stamp(path);
    if let Some(history) = cache().take(path, stamp.as_ref()) {
        return Ok(history);
    }
    let history = agents::load_session_history(harness, path, project)?;
    cache().store(path.to_path_buf(), stamp, &history);
    Ok(history)
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
