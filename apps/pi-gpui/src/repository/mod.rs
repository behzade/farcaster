//! Isolated, read-only Git and Jujutsu working-copy backend.
//!
//! Calls are synchronous by design. The UI should run `snapshot` and
//! `load_diff` off-thread and move their cloneable results back to GPUI.

mod git;
mod jj;
mod process;

use std::{
    ffi::OsString,
    fmt,
    path::{Component, Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::{Duration, SystemTime},
};

use process::{CommandOutput, CommandRunner};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
static REPOSITORY_OPERATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BackendPreference {
    #[default]
    Auto,
    Git,
    Jujutsu,
}

impl BackendPreference {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Git => "git",
            Self::Jujutsu => "jj",
        }
    }
}

impl fmt::Display for BackendPreference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for BackendPreference {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "git" => Ok(Self::Git),
            "jj" | "jujutsu" => Ok(Self::Jujutsu),
            _ => Err(format!("unknown repository backend preference: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepositoryKind {
    Git,
    Jujutsu,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepositoryLocation {
    pub(crate) kind: RepositoryKind,
    /// Absolute, canonical path containing the selected repository marker.
    pub(crate) workspace_root: PathBuf,
    /// Absolute, canonical project selected in Pi. Status is scoped here.
    pub(crate) project_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SnapshotIdentity {
    Git(GitIdentity),
    Jujutsu(JujutsuIdentity),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GitIdentity {
    pub(crate) head_oid: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) upstream: Option<String>,
    pub(crate) ahead: u64,
    pub(crate) behind: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JujutsuIdentity {
    pub(crate) operation_id: String,
    pub(crate) commit_id: String,
    pub(crate) change_id: String,
    pub(crate) description: String,
    pub(crate) bookmarks: Vec<String>,
    pub(crate) conflicted_paths: Vec<PathBuf>,
    pub(crate) conflicted: bool,
    pub(crate) empty: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ChangeLayer {
    GitIndex,
    GitWorkingTree,
    GitConflict,
    GitUntracked,
    JujutsuWorkingCopy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflict,
    Unknown(String),
}

impl ChangeKind {
    pub(crate) fn status_label(&self) -> &str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Untracked => "?",
            Self::Conflict => "U",
            Self::Unknown(status) => status,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SnapshotToken {
    Git(Arc<[u8]>),
    Jujutsu(Arc<str>),
}

/// Stable across refreshes and collision-free for non-UTF-8 paths. Display
/// strings are intentionally not part of identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DiffTargetKey {
    pub(crate) workspace_root: PathBuf,
    pub(crate) relative_path: PathBuf,
    pub(crate) layer: ChangeLayer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiffTarget {
    pub(crate) key: DiffTargetKey,
    pub(crate) workspace_root: PathBuf,
    pub(crate) relative_path: PathBuf,
    pub(crate) original_relative_path: Option<PathBuf>,
    pub(crate) layer: ChangeLayer,
    pub(crate) kind: ChangeKind,
    pub(crate) exists: bool,
    token: SnapshotToken,
}

impl DiffTarget {
    pub(crate) fn absolute_path(&self) -> PathBuf {
        self.workspace_root.join(&self.relative_path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkingCopyChange {
    pub(crate) relative_path: PathBuf,
    pub(crate) original_relative_path: Option<PathBuf>,
    pub(crate) layer: ChangeLayer,
    pub(crate) kind: ChangeKind,
    pub(crate) target: DiffTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkingCopySnapshot {
    pub(crate) location: RepositoryLocation,
    pub(crate) identity: SnapshotIdentity,
    pub(crate) changes: Vec<WorkingCopyChange>,
    pub(crate) captured_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiffResult {
    pub(crate) target: DiffTarget,
    pub(crate) patch: String,
    pub(crate) additions: Option<u64>,
    pub(crate) deletions: Option<u64>,
    pub(crate) exists: bool,
}

#[derive(Clone, Debug)]
struct RepositoryOptions {
    git_executable: OsString,
    jj_executable: OsString,
    timeout: Duration,
    output_limit: usize,
    environment: Vec<(OsString, OsString)>,
}

impl Default for RepositoryOptions {
    fn default() -> Self {
        Self {
            git_executable: std::env::var_os("PI_GUI_GIT").unwrap_or_else(|| OsString::from("git")),
            jj_executable: std::env::var_os("PI_GUI_JJ").unwrap_or_else(|| OsString::from("jj")),
            timeout: DEFAULT_TIMEOUT,
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
    /// `Auto` picks the deepest marker and prefers `.jj` only on a tie. Forced
    /// modes inspect only their marker and never fall back to the other tool.
    /// `Ok(None)` therefore means Auto found no repository; command failures
    /// occur later as `Err`, and forced missing markers are `BackendUnavailable`.
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

        let selected = find_marker(&project_root, preference)?;
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
        CommandRunner::new(
            self.options.timeout,
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

fn find_marker(
    project_root: &Path,
    preference: BackendPreference,
) -> Result<Option<(PathBuf, RepositoryKind)>, RepositoryError> {
    for ancestor in project_root.ancestors() {
        let kind = match preference {
            BackendPreference::Auto => {
                if marker_exists(&ancestor.join(".jj"))? {
                    Some(RepositoryKind::Jujutsu)
                } else if marker_exists(&ancestor.join(".git"))? {
                    Some(RepositoryKind::Git)
                } else {
                    None
                }
            }
            BackendPreference::Git => {
                marker_exists(&ancestor.join(".git"))?.then_some(RepositoryKind::Git)
            }
            BackendPreference::Jujutsu => {
                marker_exists(&ancestor.join(".jj"))?.then_some(RepositoryKind::Jujutsu)
            }
        };
        if let Some(kind) = kind {
            return Ok(Some((ancestor.to_path_buf(), kind)));
        }
    }
    Ok(None)
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

#[derive(Debug)]
pub(crate) enum RepositoryError {
    Io {
        context: String,
        source: std::io::Error,
    },
    BackendUnavailable {
        kind: RepositoryKind,
        project: PathBuf,
    },
    CommandTimedOut {
        program: String,
        timeout: Duration,
    },
    CommandFailed {
        program: String,
        status: Option<i32>,
        stderr: String,
        stderr_truncated: bool,
    },
    OutputTruncated {
        program: String,
    },
    ReaderStalled {
        program: String,
    },
    InvalidRepository(String),
    InvalidOutput {
        backend: RepositoryKind,
        detail: String,
    },
    InvalidPath(PathBuf),
    TargetMismatch(String),
    StaleSnapshot,
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::BackendUnavailable { kind, project } => {
                write!(
                    formatter,
                    "{kind:?} is not a repository at {}",
                    project.display()
                )
            }
            Self::CommandTimedOut { program, timeout } => {
                write!(formatter, "{program} timed out after {timeout:?}")
            }
            Self::CommandFailed {
                program,
                status,
                stderr,
                stderr_truncated,
            } => {
                let suffix = if *stderr_truncated {
                    " (truncated)"
                } else {
                    ""
                };
                write!(
                    formatter,
                    "{program} exited with {}: {stderr}{suffix}",
                    status.map_or_else(|| "a signal".to_owned(), |code| code.to_string())
                )
            }
            Self::OutputTruncated { program } => {
                write!(formatter, "{program} output exceeded the configured limit")
            }
            Self::ReaderStalled { program } => {
                write!(formatter, "{program} output pipes did not close after exit")
            }
            Self::InvalidRepository(detail) => write!(formatter, "invalid repository: {detail}"),
            Self::InvalidOutput { backend, detail } => {
                write!(formatter, "invalid {backend:?} output: {detail}")
            }
            Self::InvalidPath(path) => {
                write!(formatter, "invalid repository path: {}", path.display())
            }
            Self::TargetMismatch(detail) => write!(formatter, "diff target mismatch: {detail}"),
            Self::StaleSnapshot => write!(
                formatter,
                "working copy changed; refresh before loading diff"
            ),
        }
    }
}

impl std::error::Error for RepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
