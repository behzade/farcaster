mod adapter;
mod contract;
mod core;
mod domain;
mod git_head;

pub use adapter::watcher::{RepositoryWatchEvent, RepositoryWatcher};
pub use contract::{
    BackendPreference, ChangeKind, ChangeLayer, DiffResult, DiffTarget, DiffTargetKey, GitIdentity,
    JujutsuIdentity, RepositoryError, RepositoryKind, RepositoryLocation, RepositorySyncAction,
    SnapshotIdentity, WorkingCopyChange, WorkingCopySnapshot,
};
pub use git_head::git_head_contents;

pub use core::{
    PreferenceStore, RepositoryBackend, RepositoryEdit, RepositoryEditReview, load_preferences,
    save_preferences,
};

use core::{change, command_failed, diff_result, require_complete_stdout};
use domain::SnapshotToken;

#[cfg(test)]
use adapter::RepositoryOptions;
#[cfg(test)]
use core::patch_counts;
#[cfg(test)]
mod tests;
