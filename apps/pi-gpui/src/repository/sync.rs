use std::ffi::OsString;

use super::*;

impl RepositorySyncAction {
    pub(crate) fn is_available_for(self, identity: &SnapshotIdentity) -> bool {
        arguments(identity, self).is_ok()
    }
}

impl RepositoryBackend {
    pub(crate) fn sync(
        &self,
        snapshot: &WorkingCopySnapshot,
        action: RepositorySyncAction,
    ) -> Result<(), RepositoryError> {
        if snapshot.location != self.location {
            return Err(RepositoryError::TargetMismatch(
                "snapshot belongs to another repository".to_owned(),
            ));
        }
        let arguments = arguments(&snapshot.identity, action)?;
        let _operation = repository_operation()?;
        let output =
            self.sync_runner()
                .run(self.executable(), &arguments, &self.location.workspace_root)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_failed(self.executable(), &output))
        }
    }
}

fn arguments(
    identity: &SnapshotIdentity,
    action: RepositorySyncAction,
) -> Result<Vec<OsString>, RepositoryError> {
    match (identity, action) {
        (SnapshotIdentity::Git(identity), action) => {
            let (branch, remote, remote_branch) = git_target(identity)?;
            match action {
                RepositorySyncAction::PullOrFetch => {
                    Ok(["pull", "--ff-only", "--", remote, remote_branch]
                        .map(OsString::from)
                        .to_vec())
                }
                RepositorySyncAction::Push => Ok(vec![
                    OsString::from("push"),
                    OsString::from("--"),
                    OsString::from(remote),
                    OsString::from(format!("{branch}:{remote_branch}")),
                ]),
            }
        }
        (SnapshotIdentity::Jujutsu(_), RepositorySyncAction::PullOrFetch) => {
            Ok(["--no-pager", "--color=never", "git", "fetch"]
                .map(OsString::from)
                .to_vec())
        }
        (SnapshotIdentity::Jujutsu(identity), RepositorySyncAction::Push) => {
            let [bookmark] = identity.bookmarks.as_slice() else {
                let detail = if identity.bookmarks.is_empty() {
                    "Current JJ change has no bookmark"
                } else {
                    "Current JJ change has multiple bookmarks; choose one in a terminal"
                };
                return Err(RepositoryError::SyncUnavailable(detail.to_owned()));
            };
            Ok([
                OsString::from("--no-pager"),
                OsString::from("--color=never"),
                OsString::from("git"),
                OsString::from("push"),
                OsString::from("--bookmark"),
                OsString::from(format!("exact:{bookmark}")),
            ]
            .to_vec())
        }
    }
}

fn git_target(identity: &GitIdentity) -> Result<(&str, &str, &str), RepositoryError> {
    let branch = identity
        .branch
        .as_deref()
        .ok_or_else(|| RepositoryError::SyncUnavailable("Git HEAD is detached".to_owned()))?;
    let upstream = identity.upstream.as_deref().ok_or_else(|| {
        RepositoryError::SyncUnavailable(format!("Git branch {branch} has no upstream"))
    })?;
    let (remote, remote_branch) = split_git_upstream(upstream)?;
    Ok((branch, remote, remote_branch))
}

fn split_git_upstream(upstream: &str) -> Result<(&str, &str), RepositoryError> {
    upstream
        .split_once('/')
        .filter(|(remote, branch)| !remote.is_empty() && !branch.is_empty())
        .ok_or_else(|| RepositoryError::InvalidOutput {
            backend: RepositoryKind::Git,
            detail: format!("invalid upstream name: {upstream}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_sync_preserves_branch_and_upstream_mapping() {
        let identity = SnapshotIdentity::Git(GitIdentity {
            branch: Some("feature/sidebar".into()),
            upstream: Some("origin/review/sidebar".into()),
            ..GitIdentity::default()
        });

        assert_eq!(
            strings(&identity, RepositorySyncAction::PullOrFetch),
            ["pull", "--ff-only", "--", "origin", "review/sidebar"]
        );
        assert_eq!(
            strings(&identity, RepositorySyncAction::Push),
            ["push", "--", "origin", "feature/sidebar:review/sidebar"]
        );
    }

    #[test]
    fn jj_push_requires_one_exact_bookmark() {
        let mut identity = jj_identity(vec!["feature/*".into()]);
        assert_eq!(
            strings(&identity, RepositorySyncAction::Push),
            [
                "--no-pager",
                "--color=never",
                "git",
                "push",
                "--bookmark",
                "exact:feature/*"
            ]
        );

        identity = jj_identity(Vec::new());
        assert!(!RepositorySyncAction::Push.is_available_for(&identity));
        identity = jj_identity(vec!["first".into(), "second".into()]);
        assert!(!RepositorySyncAction::Push.is_available_for(&identity));
    }

    fn jj_identity(bookmarks: Vec<String>) -> SnapshotIdentity {
        SnapshotIdentity::Jujutsu(JujutsuIdentity {
            operation_id: "operation".into(),
            commit_id: "commit".into(),
            change_id: "change".into(),
            description: String::new(),
            bookmarks,
            conflicted_paths: Vec::new(),
            conflicted: false,
            empty: false,
        })
    }

    fn strings(identity: &SnapshotIdentity, action: RepositorySyncAction) -> Vec<String> {
        arguments(identity, action)
            .expect("sync arguments")
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }
}
