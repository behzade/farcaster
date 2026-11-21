use std::{
    path::Path,
    sync::{Arc, Condvar, Mutex, OnceLock},
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

#[derive(Clone, Eq, PartialEq, serde::Serialize)]
pub(super) struct FileStamp {
    modified: SystemTime,
    len: u64,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl FileStamp {
    pub(super) fn local_revision(path: &Path) -> Option<serde_json::Value> {
        let path = path.canonicalize().ok()?;
        let stamp = Self::read(&path)?;
        serde_json::to_value((path, stamp)).ok()
    }

    pub(super) fn read(path: &Path) -> Option<Self> {
        if !path.is_absolute() {
            return None;
        }
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Some(Self {
            #[cfg(unix)]
            identity: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
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

struct InFlight<K, S, T> {
    source: Arc<Source<K, S>>,
    page: OnceLock<crate::HistoryPage>,
    history: OnceLock<Arc<T>>,
}

struct State<K, S, T> {
    entries: Vec<Entry<K, S, T>>,
    loading: Vec<Arc<InFlight<K, S, T>>>,
}

pub(super) struct HistoryCache<K, S, T> {
    state: Mutex<State<K, S, T>>,
    finished: Condvar,
}

struct Loading<'a, K, S, T> {
    cache: &'a HistoryCache<K, S, T>,
    flight: Arc<InFlight<K, S, T>>,
}

impl<K, S, T> Drop for Loading<'_, K, S, T> {
    fn drop(&mut self) {
        let mut state = self.cache.state.lock().unwrap_or_else(|p| p.into_inner());
        state
            .loading
            .retain(|flight| !Arc::ptr_eq(flight, &self.flight));
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
        self.load_with_preview(key, revision, None, |_| load())
    }

    /// Each caller receives at most one page, even if a revision change triggers a retry.
    /// Only the full history enters the LRU.
    pub(super) fn load_with_preview(
        &self,
        key: K,
        revision: impl Fn() -> Option<S>,
        mut preview: Option<&mut dyn FnMut(crate::HistoryPage)>,
        load: impl FnOnce(&mut dyn FnMut(crate::HistoryPage)) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut joined: Option<Arc<InFlight<K, S, T>>> = None;
        let flight = {
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
                // A waiter can still use its completed read after LRU eviction,
                // including histories too large to retain in the cache at all.
                if let Some(flight) = joined.as_ref()
                    && flight.source.revision == before
                    && let Some(history) = flight.history.get()
                {
                    return Ok((**history).clone());
                }
                if let Some(flight) = state
                    .loading
                    .iter()
                    .find(|flight| flight.source.key == key && flight.source.revision == before)
                    .cloned()
                {
                    joined = Some(flight.clone());
                    if let Some(page) = flight.page.get()
                        && let Some(publish) = preview.take()
                    {
                        drop(state);
                        publish(page.clone());
                        state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                        continue;
                    }
                    state = self.finished.wait(state).unwrap_or_else(|p| p.into_inner());
                    continue;
                }
                let flight = Arc::new(InFlight {
                    source: Arc::new(Source {
                        key,
                        revision: before,
                    }),
                    page: OnceLock::new(),
                    history: OnceLock::new(),
                });
                state.loading.push(flight.clone());
                break Some(flight);
            }
        };
        let _loading = flight.as_ref().map(|flight| Loading {
            cache: self,
            flight: flight.clone(),
        });

        let history = load(&mut |page| {
            let Some(flight) = flight.as_ref() else {
                if let Some(publish) = preview.take() {
                    publish(page);
                }
                return;
            };
            let page = {
                // Publish under the waiter's mutex so it cannot miss the wakeup.
                let _state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                let page = flight.page.get_or_init(|| page);
                self.finished.notify_all();
                page
            };
            if let Some(publish) = preview.take() {
                publish(page.clone());
            }
        })?;
        let bytes = message_bytes(history.messages());
        if let Some(flight) = flight.as_ref() {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let source = &flight.source;
            if revision().as_ref() == Some(&source.revision) {
                let cached = Arc::new(history.clone());
                let _ = flight.history.set(cached.clone());
                if bytes > MAX_MESSAGE_BYTES {
                    return Ok(history);
                }
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
