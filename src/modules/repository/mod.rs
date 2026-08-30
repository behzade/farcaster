mod adapter;
mod contract;
mod domain;
mod sync;

pub(crate) use adapter::watcher;
pub(crate) use contract::{
    BackendPreference, ChangeKind, ChangeLayer, DiffResult, DiffTarget, DiffTargetKey, GitIdentity,
    JujutsuIdentity, RepositoryError, RepositoryKind, RepositoryLocation, RepositorySyncAction,
    SnapshotIdentity, WorkingCopyChange, WorkingCopySnapshot,
};

use domain::SnapshotToken;

use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
    time::Duration,
};

use adapter::{
    git, jj,
    process::{CommandOutput, CommandRunner},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_SYNC_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
static REPOSITORY_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug)]
struct RepositoryOptions {
    git_executable: OsString,
    jj_executable: OsString,
    timeout: Duration,
    sync_timeout: Duration,
    output_limit: usize,
    environment: Vec<(OsString, OsString)>,
}

impl Default for RepositoryOptions {
    fn default() -> Self {
        Self {
            git_executable: std::env::var_os("FARCASTER_GIT")
                .unwrap_or_else(|| OsString::from("git")),
            jj_executable: std::env::var_os("FARCASTER_JJ").unwrap_or_else(|| OsString::from("jj")),
            timeout: DEFAULT_TIMEOUT,
            sync_timeout: DEFAULT_SYNC_TIMEOUT,
            output_limit: DEFAULT_OUTPUT_LIMIT,
            environment: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RepositoryBackend {
    location: RepositoryLocation,
    options: RepositoryOptions,
}

impl RepositoryBackend {
    pub(crate) fn discover(
        project: &Path,
        preference: BackendPreference,
    ) -> Result<Option<Self>, RepositoryError> {
        Self::discover_with_options(project, preference, RepositoryOptions::default())
    }

    fn discover_with_options(
        project: &Path,
        preference: BackendPreference,
        options: RepositoryOptions,
    ) -> Result<Option<Self>, RepositoryError> {
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

        let git_available = executable_available(&options.git_executable);
        let jj_available = executable_available(&options.jj_executable);
        let preference = available_preference(preference, git_available, jj_available);
        let selected = find_marker(&project_root, preference, git_available, jj_available)?;
        let Some((workspace_root, kind)) = selected else {
            return match preference {
                BackendPreference::Auto => Ok(None),
                BackendPreference::Git => Err(RepositoryError::BackendUnavailable {
                    kind: RepositoryKind::Git,
                    project: project_root,
                }),
                BackendPreference::Jujutsu => Err(RepositoryError::BackendUnavailable {
                    kind: RepositoryKind::Jujutsu,
                    project: project_root,
                }),
            };
        };
        Ok(Some(Self {
            location: RepositoryLocation {
                kind,
                workspace_root,
                project_root,
            },
            options,
        }))
    }

    pub(crate) fn available_backends() -> (bool, bool) {
        let options = RepositoryOptions::default();
        (
            executable_available(&options.git_executable),
            executable_available(&options.jj_executable),
        )
    }

    pub(crate) fn jj_init_required(location: &RepositoryLocation) -> Result<bool, RepositoryError> {
        Ok(location.kind == RepositoryKind::Git
            && !marker_exists(&location.workspace_root.join(".jj"))?)
    }

    pub(crate) fn init_jj_colocated(repository: &Path) -> Result<(), RepositoryError> {
        Self::init_jj_colocated_with_options(repository, RepositoryOptions::default())
    }

    fn init_jj_colocated_with_options(
        repository: &Path,
        options: RepositoryOptions,
    ) -> Result<(), RepositoryError> {
        if !marker_exists(&repository.join(".git"))? {
            return Err(RepositoryError::BackendUnavailable {
                kind: RepositoryKind::Git,
                project: repository.to_path_buf(),
            });
        }
        let _operation = repository_operation()?;
        let runner = CommandRunner::new(
            options.timeout,
            options.output_limit,
            options.environment.clone(),
        );
        let arguments = [OsString::from("git"), OsString::from("init")];
        let output = runner.run(&options.jj_executable, &arguments, repository)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_failed(&options.jj_executable, &output))
        }
    }

    pub(crate) const fn location(&self) -> &RepositoryLocation {
        &self.location
    }

    pub(crate) fn snapshot(&self) -> Result<WorkingCopySnapshot, RepositoryError> {
        let _operation = repository_operation()?;
        match self.location.kind {
            RepositoryKind::Git => git::snapshot(self),
            RepositoryKind::Jujutsu => jj::snapshot(self),
        }
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
                    let diff = git::load_diff(self, change.target.clone())?;
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
        match self.location.kind {
            RepositoryKind::Git => git::load_diff(self, target),
            RepositoryKind::Jujutsu => jj::load_diff(self, target),
        }
    }

    pub(crate) fn list_project_files(&self) -> Result<Vec<String>, RepositoryError> {
        let _operation = repository_operation()?;
        match self.location.kind {
            RepositoryKind::Git => git::list_project_files(self),
            RepositoryKind::Jujutsu => jj::list_project_files(self),
        }
    }

    fn project_relative_path(&self, path: &Path) -> Option<PathBuf> {
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

    fn project_pathspec(&self) -> PathBuf {
        self.location
            .project_root
            .strip_prefix(&self.location.workspace_root)
            .ok()
            .filter(|path| !path.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    fn runner(&self) -> CommandRunner {
        self.runner_with_timeout(self.options.timeout)
    }

    fn sync_runner(&self) -> CommandRunner {
        self.runner_with_timeout(self.options.sync_timeout)
    }

    fn runner_with_timeout(&self, timeout: Duration) -> CommandRunner {
        CommandRunner::new(
            timeout,
            self.options.output_limit,
            self.options.environment.clone(),
        )
    }

    fn executable(&self) -> &OsString {
        match self.location.kind {
            RepositoryKind::Git => &self.options.git_executable,
            RepositoryKind::Jujutsu => &self.options.jj_executable,
        }
    }

    fn run(&self, arguments: &[OsString]) -> Result<CommandOutput, RepositoryError> {
        self.runner()
            .run(self.executable(), arguments, &self.location.workspace_root)
    }

    fn run_success(&self, arguments: &[OsString]) -> Result<CommandOutput, RepositoryError> {
        let output = self.run(arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(command_failed(self.executable(), &output))
        }
    }
}

fn repository_operation() -> Result<MutexGuard<'static, ()>, RepositoryError> {
    REPOSITORY_OPERATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            RepositoryError::InvalidRepository("repository operation lock is poisoned".into())
        })
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

fn executable_available(executable: &std::ffi::OsStr) -> bool {
    let executable = Path::new(executable);
    if executable.components().count() > 1 {
        return executable.is_file();
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(executable).is_file())
    })
}

fn marker_exists(path: &Path) -> Result<bool, RepositoryError> {
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

fn command_failed(program: &std::ffi::OsStr, output: &CommandOutput) -> RepositoryError {
    RepositoryError::CommandFailed {
        program: program.to_string_lossy().into_owned(),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        stderr_truncated: output.stderr_truncated,
    }
}

fn require_complete_stdout(
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

fn change(
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

fn diff_result(target: DiffTarget, patch: String) -> DiffResult {
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

fn patch_counts(patch: &str) -> (Option<u64>, Option<u64>) {
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

#[cfg(test)]
pub(crate) mod tests;
