use std::{ffi::OsString, process::ExitStatus};

use super::super::{
    DiffResult, DiffTarget, RepositoryBackend, RepositoryError, WorkingCopySnapshot,
};

#[derive(Debug)]
pub struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Clone, Copy)]
pub enum CommandMode {
    Query,
    Synchronization,
}

pub trait CommandExecutor: Send + Sync {
    fn executable(&self) -> &std::ffi::OsStr;

    fn run(
        &self,
        arguments: &[OsString],
        mode: CommandMode,
    ) -> Result<CommandOutput, RepositoryError>;
}

pub trait RepositoryOperations: Send + Sync {
    fn edit(
        &self,
        backend: &RepositoryBackend,
        review: &super::RepositoryEditReview,
        action: super::RepositoryEdit,
        message: &str,
    ) -> Result<(), RepositoryError>;

    fn snapshot(&self, backend: &RepositoryBackend)
    -> Result<WorkingCopySnapshot, RepositoryError>;

    fn load_diff(
        &self,
        backend: &RepositoryBackend,
        target: DiffTarget,
    ) -> Result<DiffResult, RepositoryError>;

    fn list_project_files(
        &self,
        backend: &RepositoryBackend,
    ) -> Result<Vec<String>, RepositoryError>;
}
