use std::{ffi::OsString, path::PathBuf};

use super::super::command_failed;
use crate::repository::{
    ChangeLayer, RepositoryBackend, RepositoryEdit, RepositoryEditReview, RepositoryError,
    SnapshotIdentity,
};

pub(super) fn apply(
    backend: &RepositoryBackend,
    review: &RepositoryEditReview,
    action: RepositoryEdit,
    message: &str,
) -> Result<(), RepositoryError> {
    let untracked = review
        .snapshot
        .changes
        .iter()
        .filter(|change| {
            change.layer == ChangeLayer::GitUntracked
                && review.paths.contains(&change.relative_path)
        })
        .map(|change| change.relative_path.clone())
        .collect::<Vec<_>>();
    match action {
        RepositoryEdit::Commit => {
            // --only commits current contents of these paths and preserves unrelated staged files.
            // Git requires new paths to be known to the index first. Intent-to-add stores no content.
            if !untracked.is_empty() {
                run(backend, &["add", "--intent-to-add"], &untracked)?;
            }
            run(backend, &["commit", "--only", "-m", message], &review.paths)
        }
        RepositoryEdit::Discard => {
            let tracked = review
                .paths
                .iter()
                .filter(|path| !untracked.contains(path))
                .cloned()
                .collect::<Vec<_>>();
            if !tracked.is_empty() {
                let unborn = matches!(&review.snapshot.identity, SnapshotIdentity::Git(identity) if identity.head_oid.is_none());
                let args: &[&str] = if unborn {
                    &["rm", "-f"]
                } else {
                    &["restore", "--source=HEAD", "--staged", "--worktree"]
                };
                run(backend, args, &tracked)?;
            }
            for path in untracked {
                // Never recursively delete a directory, ignored files, or submodule contents.
                std::fs::remove_file(backend.location.workspace_root.join(&path)).map_err(
                    |source| RepositoryError::Io {
                        context: format!("Delete {}", path.display()),
                        source,
                    },
                )?;
            }
            Ok(())
        }
    }
}

fn run(
    backend: &RepositoryBackend,
    command: &[&str],
    paths: &[PathBuf],
) -> Result<(), RepositoryError> {
    let mut args = ["--no-pager", "--literal-pathspecs"]
        .map(OsString::from)
        .to_vec();
    args.extend(command.iter().map(OsString::from));
    args.push("--".into());
    args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    let output = backend.run_sync(&args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failed(backend.executable(), &output))
    }
}
