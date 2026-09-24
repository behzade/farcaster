//! What a panel last showed, so it can paint that again instead of an empty
//! panel while the real read runs.
//!
//! Only reads of a slow source belong here: a working copy scan, a session
//! transcript, a catalog. Selection, scroll, focus, and dialog state are
//! per-visit by definition and must never be remembered.
//!
//! A remembered value always carries the [`Stamp`] it was read from, and a
//! value that cannot be stamped is never reused. A hit counts as a use, so the
//! entries a panel keeps coming back to are the ones that survive the limit.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// What a file looked like when its value was read. A source that moved on
/// must not be served from memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

pub struct Remembered<T> {
    entries: Vec<(PathBuf, Stamp, T)>,
    limit: usize,
}

impl<T: Clone> Remembered<T> {
    pub const fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit,
        }
    }

    /// The value remembered for `key`, if `key` still carries `stamp`.
    pub fn recall(&mut self, key: &Path, stamp: Option<&Stamp>) -> Option<T> {
        let index = self.index(key, stamp)?;
        let entry = self.entries.remove(index);
        let value = entry.2.clone();
        self.entries.push(entry);
        Some(value)
    }

    pub fn contains(&self, key: &Path, stamp: Option<&Stamp>) -> bool {
        self.index(key, stamp).is_some()
    }

    pub fn remember(&mut self, key: PathBuf, stamp: Option<Stamp>, value: &T) {
        self.entries.retain(|(remembered, _, _)| remembered != &key);
        let Some(stamp) = stamp else {
            return;
        };
        self.entries.push((key, stamp, value.clone()));
        if self.entries.len() > self.limit {
            self.entries.remove(0);
        }
    }

    pub fn forget(&mut self, key: &Path) {
        self.entries.retain(|(remembered, _, _)| remembered != key);
    }

    fn index(&self, key: &Path, stamp: Option<&Stamp>) -> Option<usize> {
        let stamp = stamp?;
        self.entries
            .iter()
            .position(|(remembered, remembered_stamp, _)| {
                remembered.as_path() == key && remembered_stamp == stamp
            })
    }
}

#[cfg(test)]
#[path = "remembered_tests.rs"]
mod tests;
