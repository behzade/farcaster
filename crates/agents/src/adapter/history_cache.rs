use std::{
    path::Path,
    sync::{Arc, Condvar, Mutex},
    time::SystemTime,
};

const LIMIT: usize = 24;

#[derive(Clone, Eq, PartialEq)]
pub(super) struct FileStamp {
    modified: SystemTime,
    len: u64,
}

impl FileStamp {
    pub(super) fn read(path: &Path) -> Option<Self> {
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

struct Source<K, S> {
    key: K,
    revision: S,
}

struct Entry<K, S, T> {
    source: Arc<Source<K, S>>,
    history: Arc<T>,
}

struct State<K, S, T> {
    entries: Vec<Entry<K, S, T>>,
    loading: Vec<Arc<Source<K, S>>>,
}

pub(super) struct HistoryCache<K, S, T> {
    state: Mutex<State<K, S, T>>,
    finished: Condvar,
}

struct Loading<'a, K, S, T> {
    cache: &'a HistoryCache<K, S, T>,
    source: Arc<Source<K, S>>,
}

impl<K, S, T> Drop for Loading<'_, K, S, T> {
    fn drop(&mut self) {
        let mut state = self.cache.state.lock().unwrap_or_else(|p| p.into_inner());
        state
            .loading
            .retain(|source| !Arc::ptr_eq(source, &self.source));
        self.cache.finished.notify_all();
    }
}

impl<K: Eq, S: Eq, T: Clone> HistoryCache<K, S, T> {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                entries: Vec::new(),
                loading: Vec::new(),
            }),
            finished: Condvar::new(),
        }
    }

    pub(super) fn load(
        &self,
        key: K,
        revision: impl Fn() -> Option<S>,
        load: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let source = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                let before = revision();
                if let Some(index) = state
                    .entries
                    .iter()
                    .position(|entry| entry.source.key == key)
                {
                    let entry = state.entries.remove(index);
                    if Some(&entry.source.revision) == before.as_ref() {
                        let history = entry.history.clone();
                        state.entries.push(entry);
                        drop(state);
                        return Ok((*history).clone());
                    }
                }
                let Some(before) = before else { break None };
                if state
                    .loading
                    .iter()
                    .any(|source| source.key == key && source.revision == before)
                {
                    state = self.finished.wait(state).unwrap_or_else(|p| p.into_inner());
                    continue;
                }
                let source = Arc::new(Source {
                    key,
                    revision: before,
                });
                state.loading.push(source.clone());
                break Some(source);
            }
        };
        let _loading = source.as_ref().map(|source| Loading {
            cache: self,
            source: source.clone(),
        });

        let history = load()?;
        if let Some(source) = source.as_ref() {
            let cached = Arc::new(history.clone());
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if revision().as_ref() == Some(&source.revision) {
                state.entries.retain(|entry| entry.source.key != source.key);
                state.entries.push(Entry {
                    source: source.clone(),
                    history: cached,
                });
                if state.entries.len() > LIMIT {
                    state.entries.remove(0);
                }
            }
        }
        Ok(history)
    }
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
