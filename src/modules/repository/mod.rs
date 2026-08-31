mod adapter;
mod contract;
mod core;
mod domain;

pub(crate) use adapter::watcher::{RepositoryWatchEvent, RepositoryWatcher};
pub(crate) use contract::{
    BackendPreference, ChangeKind, ChangeLayer, DiffResult, DiffTarget, DiffTargetKey, GitIdentity,
    JujutsuIdentity, RepositoryError, RepositoryKind, RepositoryLocation, RepositorySyncAction,
    SnapshotIdentity, WorkingCopyChange, WorkingCopySnapshot,
};

pub(crate) use core::{PreferenceStore, RepositoryBackend, load_preferences, save_preferences};

use core::{change, command_failed, diff_result, require_complete_stdout};
use domain::SnapshotToken;

#[cfg(test)]
use adapter::RepositoryOptions;
#[cfg(test)]
use core::patch_counts;
#[cfg(test)]
mod tests;
