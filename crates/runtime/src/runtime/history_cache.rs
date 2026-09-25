use super::*;
use super::remembered::{Remembered, Stamp};
use std::sync::{Mutex, MutexGuard};

const LIMIT: usize = 24;

static CACHE: Mutex<Remembered<LoadedHistory>> = Mutex::new(Remembered::new(LIMIT));

fn cache() -> MutexGuard<'static, Remembered<LoadedHistory>> {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn history_is_fresh(path: &std::path::Path) -> bool {
    let Some(stamp) = Stamp::of(path) else {
        return false;
    };
    cache().contains(path, Some(&stamp))
}

pub fn load_cached_history(
    harness: Backend,
    path: &std::path::Path,
    project: &std::path::Path,
) -> Result<LoadedHistory, String> {
    let stamp = Stamp::of(path);
    if let Some(history) = cache().recall(path, stamp.as_ref()) {
        return Ok(history);
    }
    let history = agents::load_session_history(harness, path, project)?;
    cache().remember(path.to_path_buf(), stamp, &history);
    Ok(history)
}

#[cfg(test)]
#[path = "history_cache_tests.rs"]
mod tests;
