//! The MCP response is a compatibility path; the local journal owns delivery.
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Submission {
    pub(crate) id: String,
    pub(crate) artifact: Value,
    pub(crate) prompt_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) user_ordinal: Option<usize>,
}

#[derive(Default)]
struct Updates {
    revision: AtomicU64,
    listeners: Mutex<Vec<Weak<std::thread::Thread>>>,
}

fn updates() -> &'static Updates {
    static UPDATES: OnceLock<Updates> = OnceLock::new();
    UPDATES.get_or_init(Updates::default)
}

pub(crate) fn revision() -> u64 {
    updates().revision.load(Ordering::Acquire)
}

pub(crate) fn subscribe() -> Arc<std::thread::Thread> {
    let thread = Arc::new(std::thread::current());
    let mut listeners = updates()
        .listeners
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    listeners.retain(|listener| listener.strong_count() > 0);
    listeners.push(Arc::downgrade(&thread));
    thread
}

pub(crate) fn notify() {
    updates().revision.fetch_add(1, Ordering::Release);
    let mut listeners = updates()
        .listeners
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    listeners.retain(|listener| {
        if let Some(thread) = listener.upgrade() {
            thread.unpark();
            true
        } else {
            false
        }
    });
}
