use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Stamp {
    modified: SystemTime,
    len: u64,
}

impl Stamp {
    pub(super) fn of(path: &Path) -> Option<Self> {
        if !path.is_absolute() {
            return None;
        }
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
        })
    }
}

pub(super) struct Remembered<T> {
    entries: Vec<(PathBuf, Stamp, T)>,
    limit: usize,
}

impl<T: Clone> Remembered<T> {
    pub(super) const fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit,
        }
    }

    pub(super) fn recall(&mut self, key: &Path, stamp: Option<&Stamp>) -> Option<T> {
        let index = self.index(key, stamp)?;
        let entry = self.entries.remove(index);
        let value = entry.2.clone();
        self.entries.push(entry);
        Some(value)
    }

    #[cfg(test)]
    pub(super) fn contains(&self, key: &Path, stamp: Option<&Stamp>) -> bool {
        self.index(key, stamp).is_some()
    }

    pub(super) fn remember(&mut self, key: PathBuf, stamp: Option<Stamp>, value: &T) {
        self.entries.retain(|(remembered, _, _)| remembered != &key);
        let Some(stamp) = stamp else {
            return;
        };
        self.entries.push((key, stamp, value.clone()));
        if self.entries.len() > self.limit {
            self.entries.remove(0);
        }
    }

    pub(super) fn forget(&mut self, key: &Path) {
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
