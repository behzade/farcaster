use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn split_patch_counts_preserve_concatenated_semantics() {
    let cases: &[&[u8]] = &[
        b"",
        b"+added\n-deleted\n",
        b"+++ b/file\n--- a/file\n+++content\n---content\n",
        b"+added\r\n-deleted",
        b"GIT binary patch\nliteral 1\n",
        b"+Binary files a and b differ\n",
        b"Binary file x",
        b"+\xf0\x9f\x99\x82\n-\xff",
    ];
    for bytes in cases {
        let expected = patch_counts(&String::from_utf8_lossy(bytes));
        for split in 0..=bytes.len() {
            assert_eq!(
                patch_counts_many(&[&bytes[..split], &[], &bytes[split..]]),
                expected,
                "split at {split}: {bytes:?}"
            );
        }
    }
}

#[test]
fn patch_totals_preserve_both_layers_and_binary_unknowns() {
    let staged = b"--- a/file\n+++ b/file\n-removed\n+++content\n";
    let working = b"--- a/file\n+++ b/file\n+added\n---content\n";
    assert_eq!(patch_counts_many(&[staged, working]), (Some(1), Some(1)));
    assert_eq!(
        patch_counts_many(&[staged, b"Binary files a/blob and b/blob differ\n"]),
        (None, None)
    );
}

struct NoCommands;

impl CommandExecutor for NoCommands {
    fn executable(&self) -> &std::ffi::OsStr {
        std::ffi::OsStr::new("unused")
    }

    fn run(&self, _: &[OsString], _: CommandMode) -> Result<CommandOutput, RepositoryError> {
        panic!("test operations must not execute commands")
    }
}

struct ObservationOperations {
    lock: Arc<Mutex<()>>,
    calls: AtomicUsize,
    snapshot_error: bool,
    totals_error: bool,
}

impl RepositoryOperations for ObservationOperations {
    fn snapshot(
        &self,
        backend: &RepositoryBackend,
    ) -> Result<WorkingCopySnapshot, RepositoryError> {
        assert!(matches!(
            self.lock.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.snapshot_error {
            return Err(RepositoryError::InvalidRepository("snapshot failed".into()));
        }
        Ok(WorkingCopySnapshot {
            location: backend.location.clone(),
            identity: crate::SnapshotIdentity::Git(Default::default()),
            changes: Vec::new(),
            captured_at: std::time::SystemTime::UNIX_EPOCH,
        })
    }

    fn working_copy_totals(
        &self,
        _: &RepositoryBackend,
        snapshot: &mut WorkingCopySnapshot,
    ) -> Result<(Option<u64>, Option<u64>), RepositoryError> {
        assert!(matches!(
            self.lock.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.totals_error {
            return Err(RepositoryError::InvalidRepository("totals failed".into()));
        }
        snapshot.captured_at += std::time::Duration::from_secs(1);
        Ok((Some(3), Some(2)))
    }

    fn sync_arguments(
        &self,
        _: &crate::SnapshotIdentity,
        _: crate::RepositorySyncAction,
    ) -> Result<Vec<OsString>, RepositoryError> {
        unreachable!()
    }

    fn edit(
        &self,
        _: &RepositoryBackend,
        _: &RepositoryEditReview,
        _: RepositoryEdit,
        _: &str,
    ) -> Result<(), RepositoryError> {
        unreachable!()
    }

    fn load_diff(
        &self,
        _: &RepositoryBackend,
        _: DiffTarget,
    ) -> Result<DiffResult, RepositoryError> {
        unreachable!()
    }

    fn list_project_files(&self, _: &RepositoryBackend) -> Result<Vec<String>, RepositoryError> {
        unreachable!()
    }
}

fn backend(
    snapshot_error: bool,
    totals_error: bool,
) -> (RepositoryBackend, Arc<ObservationOperations>) {
    let operations = Arc::new(ObservationOperations {
        lock: Arc::new(Mutex::new(())),
        calls: AtomicUsize::new(0),
        snapshot_error,
        totals_error,
    });
    let backend = RepositoryBackend::new(
        RepositoryLocation {
            kind: RepositoryKind::Git,
            workspace_root: PathBuf::from("/workspace"),
            project_root: PathBuf::from("/workspace"),
        },
        Arc::new(NoCommands),
        operations.clone(),
    );
    (backend, operations)
}

#[test]
fn busy_observation_returns_without_checking_authorization_or_running_operations() {
    let (backend, operations) = backend(false, false);
    let _held = operations.lock.lock().unwrap();
    assert!(
        backend
            .try_snapshot_with_totals_using(&operations.lock, || panic!("lock is busy"))
            .unwrap()
            .is_none()
    );
    assert_eq!(operations.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn denied_observation_checks_authorization_under_lock_and_releases_it() {
    let (backend, operations) = backend(false, false);
    assert!(
        backend
            .try_snapshot_with_totals_using(&operations.lock, || {
                assert!(matches!(
                    operations.lock.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                false
            })
            .unwrap()
            .is_none()
    );
    assert_eq!(operations.calls.load(Ordering::Relaxed), 0);
    assert!(operations.lock.try_lock().is_ok());
}

#[test]
fn allowed_observation_keeps_lock_through_snapshot_and_totals() {
    let (backend, operations) = backend(false, false);
    let (snapshot, additions, deletions) = backend
        .try_snapshot_with_totals_using(&operations.lock, || {
            assert!(matches!(
                operations.lock.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            true
        })
        .unwrap()
        .unwrap();
    assert_eq!((additions, deletions), (Some(3), Some(2)));
    assert_eq!(
        snapshot.captured_at,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1)
    );
    assert_eq!(operations.calls.load(Ordering::Relaxed), 2);
    assert!(operations.lock.try_lock().is_ok());
}

#[test]
fn snapshot_error_releases_lock_and_skips_totals() {
    let (backend, operations) = backend(true, false);
    assert!(
        backend
            .try_snapshot_with_totals_using(&operations.lock, || true)
            .is_err()
    );
    assert_eq!(operations.calls.load(Ordering::Relaxed), 1);
    assert!(operations.lock.try_lock().is_ok());
}

#[test]
fn totals_error_preserves_snapshot_with_unknown_counts_and_releases_lock() {
    let (backend, operations) = backend(false, true);
    let (snapshot, additions, deletions) = backend
        .try_snapshot_with_totals_using(&operations.lock, || true)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.location, backend.location);
    assert_eq!((additions, deletions), (None, None));
    assert_eq!(operations.calls.load(Ordering::Relaxed), 2);
    assert!(operations.lock.try_lock().is_ok());
}
