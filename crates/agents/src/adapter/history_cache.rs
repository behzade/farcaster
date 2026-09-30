use std::{
    path::Path,
    sync::{Arc, Condvar, Mutex},
    time::SystemTime,
};

const LIMIT: usize = 24;
// Bound the retained message payload as well as the number of histories.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub(super) trait CachedHistory: Clone {
    fn messages(&self) -> &Vec<serde_json::Value>;
}

impl CachedHistory for crate::DiscoveredHistory {
    fn messages(&self) -> &Vec<serde_json::Value> {
        &self.messages
    }
}

impl CachedHistory for farcaster_sessions::LoadedHistory {
    fn messages(&self) -> &Vec<serde_json::Value> {
        &self.messages
    }
}

fn message_bytes(messages: &Vec<serde_json::Value>) -> usize {
    use serde_json::Value;

    fn heap_bytes(value: &Value) -> usize {
        match value {
            Value::String(text) => text.capacity(),
            Value::Array(values) => message_bytes(values),
            Value::Object(values) => values
                .iter()
                .map(|(key, value)| {
                    key.capacity()
                        + std::mem::size_of::<(String, Value)>()
                        + 3 * std::mem::size_of::<usize>()
                        + heap_bytes(value)
                })
                .sum(),
            _ => 0,
        }
    }

    messages.capacity() * std::mem::size_of::<Value>()
        + messages.iter().map(heap_bytes).sum::<usize>()
}

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
    bytes: usize,
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

impl<K: Eq, S: Eq, T: CachedHistory> HistoryCache<K, S, T> {
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
        let bytes = message_bytes(history.messages());
        if bytes <= MAX_MESSAGE_BYTES
            && let Some(source) = source.as_ref()
        {
            let cached = Arc::new(history.clone());
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if revision().as_ref() == Some(&source.revision) {
                state.entries.retain(|entry| entry.source.key != source.key);
                state.entries.push(Entry {
                    source: source.clone(),
                    history: cached,
                    bytes,
                });
                while state.entries.len() > LIMIT
                    || state.entries.iter().map(|entry| entry.bytes).sum::<usize>()
                        > MAX_MESSAGE_BYTES
                {
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
