pub(super) mod git;
pub(super) mod jj;
pub(super) mod process;
pub(super) mod watcher;

use std::{ffi::OsString, path::Path, sync::Arc, time::Duration};

use self::process::ProcessExecutor;
use super::{
    BackendPreference, RepositoryBackend, RepositoryError, RepositoryKind, RepositoryLocation,
    command_failed,
    core::{
        discover_location, executable_available, marker_exists,
        port::{CommandExecutor as _, CommandMode, RepositoryOperations},
        repository_operation,
    },
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_SYNC_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct RepositoryOptions {
    pub(in crate::modules::repository) git_executable: OsString,
    pub(in crate::modules::repository) jj_executable: OsString,
    pub(in crate::modules::repository) timeout: Duration,
    pub(in crate::modules::repository) sync_timeout: Duration,
    pub(in crate::modules::repository) output_limit: usize,
    pub(in crate::modules::repository) environment: Vec<(OsString, OsString)>,
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

impl RepositoryBackend {
    pub(crate) fn discover(
        project: &Path,
        preference: BackendPreference,
    ) -> Result<Option<Self>, RepositoryError> {
        Self::discover_with_options(project, preference, RepositoryOptions::default())
    }

    pub(super) fn discover_with_options(
        project: &Path,
        preference: BackendPreference,
        options: RepositoryOptions,
    ) -> Result<Option<Self>, RepositoryError> {
        let git_available = executable_available(&options.git_executable);
        let jj_available = executable_available(&options.jj_executable);
        let Some(location) = discover_location(project, preference, git_available, jj_available)?
        else {
            return Ok(None);
        };
        let operations: Arc<dyn RepositoryOperations> = match location.kind {
            RepositoryKind::Git => Arc::new(git::GitOperations),
            RepositoryKind::Jujutsu => Arc::new(jj::JujutsuOperations),
        };
        let executable = match location.kind {
            RepositoryKind::Git => options.git_executable.clone(),
            RepositoryKind::Jujutsu => options.jj_executable.clone(),
        };
        let executor = ProcessExecutor::new(
            executable,
            location.workspace_root.clone(),
            options.timeout,
            options.sync_timeout,
            options.output_limit,
            options.environment,
        );
        Ok(Some(Self::new(location, Arc::new(executor), operations)))
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

    pub(super) fn init_jj_colocated_with_options(
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
        let arguments = [OsString::from("git"), OsString::from("init")];
        let executable = options.jj_executable.clone();
        let executor = ProcessExecutor::new(
            executable.clone(),
            repository.to_path_buf(),
            options.timeout,
            options.sync_timeout,
            options.output_limit,
            options.environment,
        );
        let output = executor.run(&arguments, CommandMode::Query)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_failed(&executable, &output))
        }
    }
}
