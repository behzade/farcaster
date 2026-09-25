use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::SystemTime,
};

const LIMIT: usize = 24;

// Detects ordinary writes, not same-size edits that restore the original mtime.
#[derive(Clone, Eq, PartialEq)]
pub(super) struct FileStamp {
    modified: SystemTime,
    len: u64,
}

impl FileStamp {
    pub(super) fn read(path: &Path) -> Option<Self> {
        // Process-wide caches must not confuse relative paths across cwd changes.
        if !path.is_absolute() {
            return None;
        }
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        Some(Self {
            modified: meta.modified().ok()?,
            len: meta.len(),
        })
    }
}

struct Entry<K, S, T> {
    key: K,
    revision: S,
    history: Arc<T>,
}

// Each adapter owns its keys and freshness evidence. Unknown revisions bypass
// caching; no file-system assumptions leak into runtime session selection.
pub(super) struct HistoryCache<K, S, T> {
    entries: Mutex<Vec<Entry<K, S, T>>>,
}

impl<K: Eq, S: Eq, T: Clone> HistoryCache<K, S, T> {
    pub(super) const fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn load(
        &self,
        key: K,
        revision: impl Fn() -> Option<S>,
        load: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let (before, cached) = {
            let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            let before = revision();
            let cached = entries
                .iter()
                .position(|entry| entry.key == key)
                .and_then(|index| {
                    let entry = entries.remove(index);
                    if Some(&entry.revision) != before.as_ref() {
                        return None;
                    }
                    let history = entry.history.clone();
                    entries.push(entry);
                    Some(history)
                });
            (before, cached)
        };
        if let Some(history) = cached {
            return Ok((*history).clone());
        }

        // Neither loading nor copying large histories holds the cache lock.
        let history = load()?;
        if let Some(before) = before {
            let cached = Arc::new(history.clone());
            let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            // Revalidate under the publication lock so a delayed older read
            // cannot replace a newer fill, or cache data changed during loading.
            if revision().as_ref() == Some(&before) {
                entries.retain(|entry| entry.key != key);
                entries.push(Entry {
                    key,
                    revision: before,
                    history: cached,
                });
                if entries.len() > LIMIT {
                    entries.remove(0);
                }
            }
        }
        Ok(history)
    }
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
