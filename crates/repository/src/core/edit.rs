use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::{RepositoryBackend, repository_operation, safe_relative_path};
use crate::{ChangeKind, RepositoryError, WorkingCopySnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryEdit {
    Commit,
    Discard,
}

#[derive(Clone, Debug)]
pub struct RepositoryEditReview {
    pub snapshot: WorkingCopySnapshot,
    pub paths: Vec<PathBuf>,
    selected: BTreeSet<PathBuf>,
}

impl RepositoryEditReview {
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn removes_file(&self) -> bool {
        self.snapshot.changes.iter().any(|change| {
            self.paths.contains(&change.relative_path)
                && matches!(change.kind, ChangeKind::Added | ChangeKind::Untracked)
        })
    }
}

impl RepositoryBackend {
    pub fn prepare_edit(
        &self,
        selected: &BTreeSet<PathBuf>,
    ) -> Result<RepositoryEditReview, RepositoryError> {
        let _operation = repository_operation()?;
        if selected.is_empty() {
            return Err(RepositoryError::InvalidRepository(
                "Select at least one changed file".into(),
            ));
        }
        self.prepare_edit_from_snapshot(self.operations.snapshot(self)?, selected)
    }

    fn prepare_edit_from_snapshot(
        &self,
        snapshot: WorkingCopySnapshot,
        selected: &BTreeSet<PathBuf>,
    ) -> Result<RepositoryEditReview, RepositoryError> {
        for path in selected {
            self.validate_edit_path(path)?;
        }
        let mut paths = selected
            .iter()
            .filter(|path| {
                snapshot
                    .changes
                    .iter()
                    .any(|change| &change.relative_path == *path)
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        if paths.is_empty() {
            return Err(RepositoryError::InvalidRepository(
                "Selected files have no changes".into(),
            ));
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
        for path in &paths {
            self.validate_edit_path(path)?;
        }
        Ok(RepositoryEditReview {
            snapshot,
            paths,
            selected: selected.clone(),
        })
    }

    pub fn apply_edit(
        &self,
        review: &RepositoryEditReview,
        action: RepositoryEdit,
        message: &str,
    ) -> Result<(), RepositoryError> {
        let _operation = repository_operation()?;
        if review.snapshot.location != self.location || review.selected.is_empty() {
            return Err(RepositoryError::TargetMismatch(
                "Review belongs to another repository".into(),
            ));
        }
        if action == RepositoryEdit::Commit && message.trim().is_empty() {
            return Err(RepositoryError::InvalidRepository(
                "Enter a commit message".into(),
            ));
        }
        let current =
            self.prepare_edit_from_snapshot(self.operations.snapshot(self)?, &review.selected)?;
        self.operations.edit(self, &current, action, message.trim())
    }

    fn validate_edit_path(&self, path: &Path) -> Result<(), RepositoryError> {
        if !safe_relative_path(path)
            || self.project_relative_path(path).is_none()
            || path
                .components()
                .any(|part| matches!(part.as_os_str().to_str(), Some(".git" | ".jj")))
        {
            return Err(RepositoryError::InvalidPath(path.to_path_buf()));
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
        let full_path = self.location.workspace_root.join(path);
        match fs::symlink_metadata(&full_path) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => Ok(()),
            Ok(_) => Err(RepositoryError::InvalidRepository(format!(
                "{} is not a regular file or symlink; handle directories and submodules in a terminal",
                full_path.display()
            ))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(RepositoryError::Io {
                context: format!("Read {} for review", full_path.display()),
                source,
            }),
        }
    }
}
