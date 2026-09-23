use super::backend::for_backend;
use crate::Backend;
use farcaster_projects::{AppliedTrust, StartupTrust, TrustChoice};
use std::path::{Path, PathBuf};

pub fn project_trust_description(backend: impl Into<Option<Backend>>) -> Option<&'static str> {
    backend
        .into()
        .and_then(|backend| for_backend(backend).trust_description())
}

pub fn project_trust(backend: Backend, project: &Path) -> Result<StartupTrust, String> {
    for_backend(backend).project_trust(project)
}

pub fn apply_project_trust(
    backend: Backend,
    project: &Path,
    choice: TrustChoice,
) -> Result<AppliedTrust, String> {
    for_backend(backend).apply_project_trust(project, choice)
}

pub fn saved_project_trust(
    backend: Backend,
    project: &Path,
) -> Result<Option<(PathBuf, bool)>, String> {
    for_backend(backend).saved_project_trust(project)
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
