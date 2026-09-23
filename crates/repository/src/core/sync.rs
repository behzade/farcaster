use super::super::{RepositoryError, RepositorySyncAction, WorkingCopySnapshot};
use super::{RepositoryBackend, command_failed, repository_operation};

impl RepositoryBackend {
    pub fn sync(
        &self,
        snapshot: &WorkingCopySnapshot,
        action: RepositorySyncAction,
    ) -> Result<(), RepositoryError> {
        if snapshot.location != self.location {
            return Err(RepositoryError::TargetMismatch(
                "snapshot belongs to another repository".to_owned(),
            ));
        }
        let arguments = self.operations.sync_arguments(&snapshot.identity, action)?;
        let _operation = repository_operation()?;
        let output = self.run_sync(&arguments)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_failed(self.executable(), &output))
        }
    }
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
