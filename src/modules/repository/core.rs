pub(super) mod port;
mod preferences;
mod sync;

pub(crate) use preferences::{PreferenceStore, load as load_preferences, save as save_preferences};

use super::{
    contract::{
        BackendPreference, ChangeKind, ChangeLayer, DiffResult, DiffTarget, DiffTargetKey,
        RepositoryError, RepositoryKind, RepositoryLocation, SnapshotIdentity, WorkingCopyChange,
        WorkingCopySnapshot,
    },
    domain::SnapshotToken,
};

use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use port::{CommandExecutor, CommandMode, CommandOutput, RepositoryOperations};

static REPOSITORY_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct RepositoryBackend {
    pub(super) location: RepositoryLocation,
    executor: Arc<dyn CommandExecutor>,
    operations: Arc<dyn RepositoryOperations>,
}

impl std::fmt::Debug for RepositoryBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RepositoryBackend")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl RepositoryBackend {
    pub(super) fn new(
        location: RepositoryLocation,
        executor: Arc<dyn CommandExecutor>,
        operations: Arc<dyn RepositoryOperations>,
    ) -> Self {
        Self {
            location,
            executor,
            operations,
        }
    }

    pub(crate) const fn location(&self) -> &RepositoryLocation {
        &self.location
    }

    pub(crate) fn snapshot(&self) -> Result<WorkingCopySnapshot, RepositoryError> {
        let _operation = repository_operation()?;
        self.operations.snapshot(self)
    }

    pub(crate) fn working_copy_totals(
        &self,
        snapshot: &WorkingCopySnapshot,
    ) -> Result<(Option<u64>, Option<u64>), RepositoryError> {
        if snapshot.location != self.location {
            return Err(RepositoryError::TargetMismatch(
                "snapshot belongs to another repository".to_owned(),
            ));
        }
        let _operation = repository_operation()?;
        let output = match &snapshot.identity {
            SnapshotIdentity::Git(_) => {
                let mut patch = Vec::new();
                for staged in [true, false] {
                    let mut arguments = [
                        "--no-pager",
                        "--no-optional-locks",
                        "--literal-pathspecs",
                        "-c",
                        "core.fsmonitor=false",
                        "diff",
                        "--no-color",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--find-renames",
                    ]
                    .map(OsString::from)
                    .to_vec();
                    if staged {
                        arguments.push(OsString::from("--cached"));
                    }
                    arguments.push(OsString::from("--"));
                    arguments.push(self.project_pathspec().into_os_string());
                    let output = self.run_success(&arguments)?;
                    require_complete_stdout(self.executable(), &output)?;
                    patch.extend(output.stdout);
                }
                for change in snapshot
                    .changes
                    .iter()
                    .filter(|change| change.layer == ChangeLayer::GitUntracked)
                {
                    let diff = self.operations.load_diff(self, change.target.clone())?;
                    patch.extend(diff.patch.into_bytes());
                }
                patch
            }
            SnapshotIdentity::Jujutsu(identity) => {
                let arguments = vec![
                    OsString::from("--no-pager"),
                    OsString::from("--color=never"),
                    OsString::from("--at-operation"),
                    OsString::from(&identity.operation_id),
                    OsString::from("diff"),
                    OsString::from("-r"),
                    OsString::from("@"),
                    OsString::from("--git"),
                    OsString::from("--"),
                    self.project_pathspec().into_os_string(),
                ];
                let output = self.run_success(&arguments)?;
                require_complete_stdout(self.executable(), &output)?;
                output.stdout
            }
        };
        Ok(patch_counts(&String::from_utf8_lossy(&output)))
    }

    #[cfg(test)]
    pub(crate) fn load_diff(&self, target: DiffTarget) -> Result<DiffResult, RepositoryError> {
        self.validate_target(&target)?;
        let _operation = repository_operation()?;
        self.operations.load_diff(self, target)
    }

    pub(crate) fn list_project_files(&self) -> Result<Vec<String>, RepositoryError> {
        let _operation = repository_operation()?;
        self.operations.list_project_files(self)
    }

    pub(super) fn project_relative_path(&self, path: &Path) -> Option<PathBuf> {
        let project = self.project_pathspec();
        if project == Path::new(".") {
            Some(path.to_path_buf())
        } else {
            path.strip_prefix(project).ok().map(Path::to_path_buf)
        }
    }

    #[cfg(test)]
    fn validate_target(&self, target: &DiffTarget) -> Result<(), RepositoryError> {
        if target.workspace_root != self.location.workspace_root {
            return Err(RepositoryError::TargetMismatch(format!(
                "target belongs to {}, backend belongs to {}",
                target.workspace_root.display(),
                self.location.workspace_root.display()
            )));
        }
        if !safe_relative_path(&target.relative_path)
            || target
                .original_relative_path
                .as_deref()
                .is_some_and(|path| !safe_relative_path(path))
        {
            return Err(RepositoryError::InvalidPath(target.relative_path.clone()));
        }
        let layer_matches = matches!(
            (self.location.kind, target.layer),
            (
                RepositoryKind::Git,
                ChangeLayer::GitIndex
                    | ChangeLayer::GitWorkingTree
                    | ChangeLayer::GitConflict
                    | ChangeLayer::GitUntracked
            ) | (RepositoryKind::Jujutsu, ChangeLayer::JujutsuWorkingCopy)
        );
        if !layer_matches {
            return Err(RepositoryError::TargetMismatch(
                "target layer does not match repository kind".to_owned(),
            ));
        }
        Ok(())
    }

    pub(super) fn project_pathspec(&self) -> PathBuf {
        self.location
            .project_root
            .strip_prefix(&self.location.workspace_root)
            .ok()
            .filter(|path| !path.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    pub(super) fn executable(&self) -> &std::ffi::OsStr {
        self.executor.executable()
    }

    pub(super) fn run(&self, arguments: &[OsString]) -> Result<CommandOutput, RepositoryError> {
        self.executor.run(arguments, CommandMode::Query)
    }

    pub(super) fn run_sync(
        &self,
        arguments: &[OsString],
    ) -> Result<CommandOutput, RepositoryError> {
        self.executor.run(arguments, CommandMode::Synchronization)
    }

    pub(super) fn run_success(
        &self,
        arguments: &[OsString],
    ) -> Result<CommandOutput, RepositoryError> {
        let output = self.run(arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(command_failed(self.executable(), &output))
        }
    }
}

pub(super) fn repository_operation() -> Result<MutexGuard<'static, ()>, RepositoryError> {
    REPOSITORY_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            RepositoryError::InvalidRepository("repository operation lock is poisoned".into())
        })
}

pub(super) fn discover_location(
    project: &Path,
    preference: BackendPreference,
    git_available: bool,
    jj_available: bool,
) -> Result<Option<RepositoryLocation>, RepositoryError> {
    let canonical = project
        .canonicalize()
        .map_err(|source| RepositoryError::Io {
            context: format!("resolve project path {}", project.display()),
            source,
        })?;
    let project_root = if canonical.is_file() {
        canonical.parent().map(Path::to_path_buf).ok_or_else(|| {
            RepositoryError::InvalidRepository(format!(
                "project path has no parent: {}",
                canonical.display()
            ))
        })?
    } else {
        canonical
    };
    let preference = available_preference(preference, git_available, jj_available);
    let Some((workspace_root, kind)) =
        find_marker(&project_root, preference, git_available, jj_available)?
    else {
        let unavailable_kind = match preference {
            BackendPreference::Auto => return Ok(None),
            BackendPreference::Git => RepositoryKind::Git,
            BackendPreference::Jujutsu => RepositoryKind::Jujutsu,
        };
        if find_marker(
            &project_root,
            BackendPreference::Auto,
            git_available,
            jj_available,
        )?
        .is_none()
        {
            return Ok(None);
        }
        return Err(RepositoryError::BackendUnavailable {
            kind: unavailable_kind,
            project: project_root,
        });
    };
    Ok(Some(RepositoryLocation {
        kind,
        workspace_root,
        project_root,
    }))
}

fn available_preference(
    preference: BackendPreference,
    git_available: bool,
    jj_available: bool,
) -> BackendPreference {
    match preference {
        BackendPreference::Git if !git_available => BackendPreference::Auto,
        BackendPreference::Jujutsu if !jj_available => BackendPreference::Auto,
        preference => preference,
    }
}

fn find_marker(
    project_root: &Path,
    preference: BackendPreference,
    git_available: bool,
    jj_available: bool,
) -> Result<Option<(PathBuf, RepositoryKind)>, RepositoryError> {
    for ancestor in project_root.ancestors() {
        let kind = match preference {
            BackendPreference::Auto => {
                if jj_available && marker_exists(&ancestor.join(".jj"))? {
                    Some(RepositoryKind::Jujutsu)
                } else if git_available && marker_exists(&ancestor.join(".git"))? {
                    Some(RepositoryKind::Git)
                } else {
                    None
                }
            }
            BackendPreference::Git => (git_available && marker_exists(&ancestor.join(".git"))?)
                .then_some(RepositoryKind::Git),
            BackendPreference::Jujutsu => (jj_available && marker_exists(&ancestor.join(".jj"))?)
                .then_some(RepositoryKind::Jujutsu),
        };
        if let Some(kind) = kind {
            return Ok(Some((ancestor.to_path_buf(), kind)));
        }
    }
    Ok(None)
}

pub(super) fn executable_available(executable: &std::ffi::OsStr) -> bool {
    let executable = Path::new(executable);
    if executable.components().count() > 1 {
        return executable.is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(executable).is_file())
    })
}

pub(super) fn marker_exists(path: &Path) -> Result<bool, RepositoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RepositoryError::Io {
            context: format!("inspect repository marker {}", path.display()),
            source,
        }),
    }
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

pub(super) fn command_failed(program: &std::ffi::OsStr, output: &CommandOutput) -> RepositoryError {
    RepositoryError::CommandFailed {
        program: program.to_string_lossy().into_owned(),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        stderr_truncated: output.stderr_truncated,
    }
}

pub(super) fn require_complete_stdout(
    program: &std::ffi::OsStr,
    output: &CommandOutput,
) -> Result<(), RepositoryError> {
    if output.stdout_truncated {
        Err(RepositoryError::OutputTruncated {
            program: program.to_string_lossy().into_owned(),
        })
    } else {
        Ok(())
    }
}

pub(super) fn change(
    location: &RepositoryLocation,
    token: SnapshotToken,
    relative_path: PathBuf,
    original_relative_path: Option<PathBuf>,
    layer: ChangeLayer,
    kind: ChangeKind,
) -> Result<WorkingCopyChange, RepositoryError> {
    if !safe_relative_path(&relative_path)
        || original_relative_path
            .as_deref()
            .is_some_and(|path| !safe_relative_path(path))
    {
        return Err(RepositoryError::InvalidPath(relative_path));
    }
    let exists = location.workspace_root.join(&relative_path).exists();
    let target = DiffTarget {
        key: DiffTargetKey {
            workspace_root: location.workspace_root.clone(),
            relative_path: relative_path.clone(),
            layer,
        },
        workspace_root: location.workspace_root.clone(),
        relative_path: relative_path.clone(),
        original_relative_path: original_relative_path.clone(),
        layer,
        kind: kind.clone(),
        exists,
        token,
    };
    Ok(WorkingCopyChange {
        relative_path,
        original_relative_path,
        layer,
        kind,
        target,
    })
}

pub(super) fn diff_result(target: DiffTarget, patch: String) -> DiffResult {
    let (additions, deletions) = patch_counts(&patch);
    let exists = target.absolute_path().exists();
    DiffResult {
        target,
        patch,
        additions,
        deletions,
        exists,
    }
}

pub(super) fn patch_counts(patch: &str) -> (Option<u64>, Option<u64>) {
    if patch.contains("GIT binary patch")
        || patch.contains("Binary files ")
        || patch.contains("Binary file ")
    {
        return (None, None);
    }
    let mut additions = 0_u64;
    let mut deletions = 0_u64;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions = additions.saturating_add(1);
        } else if line.starts_with('-') {
            deletions = deletions.saturating_add(1);
        }
    }
    (Some(additions), Some(deletions))
}
