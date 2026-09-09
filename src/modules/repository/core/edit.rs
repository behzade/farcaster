use std::{
    collections::BTreeSet,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

use super::{RepositoryBackend, repository_operation, safe_relative_path};
use crate::repository::{ChangeKind, RepositoryError, WorkingCopySnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepositoryEdit {
    Commit,
    Discard,
}

#[derive(Clone, Debug)]
pub(crate) struct RepositoryEditReview {
    pub(in crate::modules::repository) snapshot: WorkingCopySnapshot,
    pub(in crate::modules::repository) paths: Vec<PathBuf>,
    fingerprints: Vec<Option<[u8; 32]>>,
}

impl RepositoryEditReview {
    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub(crate) fn removes_file(&self) -> bool {
        self.snapshot.changes.iter().any(|change| {
            self.paths.contains(&change.relative_path)
                && matches!(change.kind, ChangeKind::Added | ChangeKind::Untracked)
        })
    }
}

impl RepositoryBackend {
    pub(crate) fn prepare_edit(
        &self,
        expected: &WorkingCopySnapshot,
        selected: &BTreeSet<PathBuf>,
    ) -> Result<RepositoryEditReview, RepositoryError> {
        let _operation = repository_operation()?;
        if expected.location != self.location || selected.is_empty() {
            return Err(RepositoryError::InvalidRepository(
                "Select at least one changed file".into(),
            ));
        }
        let snapshot = self.operations.snapshot(self)?;
        if !same_snapshot(expected, &snapshot) {
            return Err(RepositoryError::StaleSnapshot);
        }
        let mut paths = selected.clone();
        for path in selected {
            if !snapshot
                .changes
                .iter()
                .any(|change| &change.relative_path == path)
            {
                return Err(RepositoryError::StaleSnapshot);
            }
        }
        // A rename is one whole-file operation, even if Git reports its ends separately.
        loop {
            let before = paths.len();
            for change in &snapshot.changes {
                let original = change
                    .original_relative_path
                    .as_ref()
                    .filter(|_| change.kind == ChangeKind::Renamed);
                if paths.contains(&change.relative_path)
                    || original.is_some_and(|path| paths.contains(path))
                {
                    if change.kind == ChangeKind::Conflict {
                        return Err(RepositoryError::InvalidRepository(
                            "Resolve file conflicts before using this action".into(),
                        ));
                    }
                    paths.insert(change.relative_path.clone());
                    if let Some(original) = original {
                        paths.insert(original.clone());
                    }
                }
            }
            if before == paths.len() {
                break;
            }
        }
        let paths = paths.into_iter().collect::<Vec<_>>();
        let fingerprints = self.edit_fingerprints(&paths)?;
        Ok(RepositoryEditReview {
            snapshot,
            paths,
            fingerprints,
        })
    }

    pub(crate) fn apply_edit(
        &self,
        review: &RepositoryEditReview,
        action: RepositoryEdit,
        message: &str,
    ) -> Result<(), RepositoryError> {
        let _operation = repository_operation()?;
        if review.snapshot.location != self.location || review.paths.is_empty() {
            return Err(RepositoryError::TargetMismatch(
                "Review belongs to another repository".into(),
            ));
        }
        if action == RepositoryEdit::Commit && message.trim().is_empty() {
            return Err(RepositoryError::InvalidRepository(
                "Enter a commit message".into(),
            ));
        }
        let current = self.operations.snapshot(self)?;
        if !same_snapshot(&review.snapshot, &current)
            || self.edit_fingerprints(&review.paths)? != review.fingerprints
        {
            return Err(RepositoryError::StaleSnapshot);
        }
        self.operations.edit(self, review, action, message.trim())
    }

    fn edit_fingerprints(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<Option<[u8; 32]>>, RepositoryError> {
        paths
            .iter()
            .map(|path| {
                if !safe_relative_path(path)
                    || self.project_relative_path(path).is_none()
                    || path
                        .components()
                        .any(|part| matches!(part.as_os_str().to_str(), Some(".git" | ".jj")))
                {
                    return Err(RepositoryError::InvalidPath(path.clone()));
                }
                // Reject symlinked ancestors; the leaf itself may be a tracked symlink.
                let mut parent = path.parent();
                while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
                    if fs::symlink_metadata(self.location.workspace_root.join(path))
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                    {
                        return Err(RepositoryError::InvalidPath(path.to_path_buf()));
                    }
                    parent = path.parent();
                }
                fingerprint(&self.location.workspace_root.join(path))
            })
            .collect()
    }
}

fn same_snapshot(left: &WorkingCopySnapshot, right: &WorkingCopySnapshot) -> bool {
    left.location == right.location
        && left.identity == right.identity
        && left.changes.len() == right.changes.len()
        && left
            .changes
            .iter()
            .zip(&right.changes)
            .all(|(left, right)| left.target == right.target)
}

fn fingerprint(path: &Path) -> Result<Option<[u8; 32]>, RepositoryError> {
    let io_error = |source| RepositoryError::Io {
        context: format!("Read {} for review", path.display()),
        source,
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    let mut hash = Sha256::new();
    if metadata.file_type().is_symlink() {
        hash.update(b"symlink");
        hash.update(
            fs::read_link(path)
                .map_err(io_error)?
                .as_os_str()
                .as_encoded_bytes(),
        );
    } else if metadata.is_file() {
        hash.update(b"file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            hash.update(metadata.permissions().mode().to_le_bytes());
        }
        let mut file = fs::File::open(path).map_err(io_error)?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
    } else {
        return Err(RepositoryError::InvalidRepository(format!(
            "{} is not a regular file or symlink; handle directories and submodules in a terminal",
            path.display()
        )));
    }
    Ok(Some(hash.finalize().into()))
}
