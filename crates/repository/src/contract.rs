use std::{
    fmt,
    path::PathBuf,
    str::FromStr,
    time::{Duration, SystemTime},
};

use super::domain::SnapshotToken;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackendPreference {
    #[default]
    Auto,
    Git,
    Jujutsu,
}

impl BackendPreference {
    pub const fn as_str(self) -> &'static str {
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
pub enum RepositoryKind {
    Git,
    Jujutsu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositorySyncAction {
    PullOrFetch,
    Push,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryLocation {
    pub kind: RepositoryKind,
    pub workspace_root: PathBuf,
    pub project_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotIdentity {
    Git(GitIdentity),
    Jujutsu(JujutsuIdentity),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitIdentity {
    pub head_oid: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub nearest_branch: Option<String>,
    pub ahead: u64,
    pub behind: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JujutsuIdentity {
    pub operation_id: String,
    pub commit_id: String,
    pub change_id: String,
    pub description: String,
    pub bookmarks: Vec<String>,
    pub closest_bookmarks: Vec<String>,
    pub ahead: u64,
    pub conflicted_paths: Vec<PathBuf>,
    pub conflicted: bool,
    pub empty: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChangeLayer {
    GitIndex,
    GitWorkingTree,
    GitConflict,
    GitUntracked,
    JujutsuWorkingCopy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangeKind {
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
    pub fn status_label(&self) -> &str {
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DiffTargetKey {
    pub workspace_root: PathBuf,
    pub relative_path: PathBuf,
    pub layer: ChangeLayer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffTarget {
    pub key: DiffTargetKey,
    pub workspace_root: PathBuf,
    pub relative_path: PathBuf,
    pub original_relative_path: Option<PathBuf>,
    pub layer: ChangeLayer,
    pub kind: ChangeKind,
    pub exists: bool,
    pub(super) token: SnapshotToken,
}

impl DiffTarget {
    pub fn absolute_path(&self) -> PathBuf {
        self.workspace_root.join(&self.relative_path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingCopyChange {
    pub counts: Option<(usize, usize)>,
    pub relative_path: PathBuf,
    pub original_relative_path: Option<PathBuf>,
    pub layer: ChangeLayer,
    pub kind: ChangeKind,
    pub target: DiffTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkingCopySnapshot {
    pub location: RepositoryLocation,
    pub identity: SnapshotIdentity,
    pub changes: Vec<WorkingCopyChange>,
    pub captured_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffResult {
    pub target: DiffTarget,
    pub patch: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub exists: bool,
}

#[derive(Debug)]
pub enum RepositoryError {
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
    SyncUnavailable(String),
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
            Self::SyncUnavailable(detail) => formatter.write_str(detail),
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
