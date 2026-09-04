use crate::projects::{AppliedTrust, StartupTrust, TrustChoice};
use std::path::{Path, PathBuf};

pub(crate) fn project_trust_description(backend: &str) -> Option<&'static str> {
    (backend == "pi").then_some("Trusting allows Pi to load project settings and resources, install missing project packages, and execute project extensions.")
}

pub(crate) fn project_trust(backend: &str, project: &Path) -> Result<StartupTrust, String> {
    match backend {
        "pi" => super::pi::trust::startup_trust(project),
        _ => Ok(StartupTrust::Ready),
    }
}

pub(crate) fn apply_project_trust(
    backend: &str,
    project: &Path,
    choice: TrustChoice,
) -> Result<AppliedTrust, String> {
    match backend {
        "pi" => super::pi::trust::apply(project, choice),
        _ => Err(format!("{backend} manages its own project trust")),
    }
}

pub(crate) fn saved_project_trust(
    backend: &str,
    project: &Path,
) -> Result<Option<(PathBuf, bool)>, String> {
    match backend {
        "pi" => super::pi::trust::saved_decision(project),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_backends_do_not_read_pi_resources() {
        for backend in ["codex-cli", "cursor-cli", "opencode2"] {
            let nonexistent = Path::new("/nonexistent/farcaster-trust-test");
            assert_eq!(project_trust(backend, nonexistent), Ok(StartupTrust::Ready));
            assert_eq!(saved_project_trust(backend, nonexistent), Ok(None));
            assert!(project_trust_description(backend).is_none());
        }
    }
}
