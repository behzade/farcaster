use std::{ffi::OsString, process::ExitStatus};

use super::super::{
    DiffResult, DiffTarget, RepositoryBackend, RepositoryError, WorkingCopySnapshot,
};

#[derive(Debug)]
pub(in crate::modules::repository) struct CommandOutput {
    pub(in crate::modules::repository) status: ExitStatus,
    pub(in crate::modules::repository) stdout: Vec<u8>,
    pub(in crate::modules::repository) stderr: Vec<u8>,
    pub(in crate::modules::repository) stdout_truncated: bool,
    pub(in crate::modules::repository) stderr_truncated: bool,
}

#[derive(Clone, Copy)]
pub(in crate::modules::repository) enum CommandMode {
    Query,
    Synchronization,
}

pub(in crate::modules::repository) trait CommandExecutor:
    Send + Sync
{
    fn executable(&self) -> &std::ffi::OsStr;

    fn run(
        &self,
        arguments: &[OsString],
        mode: CommandMode,
    ) -> Result<CommandOutput, RepositoryError>;
}

pub(in crate::modules::repository) trait RepositoryOperations:
    Send + Sync
{
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
