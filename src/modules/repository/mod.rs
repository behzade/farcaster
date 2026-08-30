mod adapter;
mod contract;
mod core;
mod domain;
mod sync;

pub(crate) use adapter::watcher::{RepositoryWatchEvent, RepositoryWatcher};
pub(crate) use contract::{
    BackendPreference, ChangeKind, ChangeLayer, DiffResult, DiffTarget, DiffTargetKey, GitIdentity,
    JujutsuIdentity, RepositoryError, RepositoryKind, RepositoryLocation, RepositorySyncAction,
    SnapshotIdentity, WorkingCopyChange, WorkingCopySnapshot,
};

pub(crate) use core::RepositoryBackend;

use core::{change, command_failed, diff_result, repository_operation, require_complete_stdout};
use domain::SnapshotToken;
