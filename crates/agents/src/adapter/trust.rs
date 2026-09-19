use crate::Backend;
use farcaster_projects::{AppliedTrust, StartupTrust, TrustChoice};
use std::path::{Path, PathBuf};

pub fn project_trust_description(backend: impl Into<Option<Backend>>) -> Option<&'static str> {
    let backend = backend.into()?;
    (backend == Backend::Pi).then_some("Trusting allows Pi to load project settings and resources, install missing project packages, and execute project extensions.")
}

pub fn project_trust(backend: Backend, project: &Path) -> Result<StartupTrust, String> {
    match backend {
        Backend::Pi => super::pi::trust::startup_trust(project),
        Backend::Codex
        | Backend::Cursor
        | Backend::OpenCode
        | Backend::Claude
        | Backend::Antigravity => Ok(StartupTrust::Ready),
    }
}

pub fn apply_project_trust(
    backend: Backend,
    project: &Path,
    choice: TrustChoice,
) -> Result<AppliedTrust, String> {
    match backend {
        Backend::Pi => super::pi::trust::apply(project, choice),
        Backend::Codex
        | Backend::Cursor
        | Backend::OpenCode
        | Backend::Claude
        | Backend::Antigravity => Err(format!("{backend} manages its own project trust")),
    }
}

pub fn saved_project_trust(
    backend: Backend,
    project: &Path,
) -> Result<Option<(PathBuf, bool)>, String> {
    match backend {
        Backend::Pi => super::pi::trust::saved_decision(project),
        Backend::Codex
        | Backend::Cursor
        | Backend::OpenCode
        | Backend::Claude
        | Backend::Antigravity => Ok(None),
    }
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
