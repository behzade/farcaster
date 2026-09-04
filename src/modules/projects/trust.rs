use std::path::{Path, PathBuf};

use super::trust_store::{canonical, nearest_decision, update_trust_file};
use super::{AppliedTrust, StartupTrust, TrustChoice, TrustOption};

pub(crate) const TRUST_DESCRIPTION: &str = "Trusting allows Farcaster to run repository commands in this folder. Agent settings and extensions follow each backend's own trust rules.";

pub(crate) fn startup_trust(store: &Path, project: &Path) -> Result<StartupTrust, String> {
    Ok(if saved_decision(store, project)?.is_some() {
        StartupTrust::Ready
    } else {
        StartupTrust::Prompt
    })
}

pub(crate) fn repository_execution_allowed(store: &Path, project: &Path) -> Result<bool, String> {
    Ok(saved_decision(store, project)?.is_some_and(|(_, trusted)| trusted))
}

pub(crate) fn saved_decision(
    store: &Path,
    project: &Path,
) -> Result<Option<(PathBuf, bool)>, String> {
    nearest_decision(store, project)
}

pub(crate) fn options(project: &Path) -> Vec<TrustOption> {
    let mut options = vec![TrustOption {
        label: "Trust project".into(),
        choice: TrustChoice::TrustProject,
    }];
    if let Some(parent) = project.parent() {
        options.push(TrustOption {
            label: format!("Trust parent folder ({})", parent.display()),
            choice: TrustChoice::TrustParent,
        });
    }
    options.push(TrustOption {
        label: "Do not trust".into(),
        choice: TrustChoice::DistrustProject,
    });
    options
}

pub(crate) fn apply(
    store: &Path,
    project: &Path,
    choice: TrustChoice,
) -> Result<AppliedTrust, String> {
    let project = canonical(project)?;
    let trusted = choice != TrustChoice::DistrustProject;
    let scope = if choice == TrustChoice::TrustParent {
        project
            .parent()
            .ok_or_else(|| "the project has no parent folder to trust".to_owned())?
            .to_path_buf()
    } else {
        project.clone()
    };
    let mut updates = vec![(scope.clone(), Some(trusted))];
    if scope != project {
        updates.push((project, None));
    }
    update_trust_file(store, &updates)?;
    Ok(AppliedTrust {
        trusted,
        saved_path: Some(scope),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_trust_is_explicit_and_independent_of_backend_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        std::fs::create_dir_all(project.join(".pi/extensions"))?;
        let store = temp.path().join("farcaster/project-trust.json");
        assert_eq!(startup_trust(&store, &project)?, StartupTrust::Prompt);
        assert!(!repository_execution_allowed(&store, &project)?);
        apply(&store, &project, TrustChoice::TrustProject)?;
        assert!(repository_execution_allowed(&store, &project)?);
        std::fs::write(
            project.join(".pi/settings.json"),
            r#"{"defaultProjectTrust":"never"}"#,
        )?;
        assert!(repository_execution_allowed(&store, &project)?);
        assert!(!project.join(".pi/trust.json").exists());
        apply(&store, &project, TrustChoice::DistrustProject)?;
        assert_eq!(startup_trust(&store, &project)?, StartupTrust::Ready);
        assert!(!repository_execution_allowed(&store, &project)?);
        Ok(())
    }

    #[test]
    fn parent_trust_replaces_a_child_decision_and_survives_reopening()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("group/project");
        std::fs::create_dir_all(&project)?;
        let store = temp.path().join("trust.json");
        apply(&store, &project, TrustChoice::DistrustProject)?;
        apply(&store, &project, TrustChoice::TrustParent)?;
        assert_eq!(
            saved_decision(&store, &project)?,
            Some((project.parent().expect("parent").canonicalize()?, true))
        );
        Ok(())
    }

    #[test]
    fn malformed_trust_cannot_allow_execution() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("trust.json");
        std::fs::write(&store, r#"{"/project":"yes"}"#)?;
        assert!(repository_execution_allowed(&store, temp.path()).is_err());
        Ok(())
    }
}
