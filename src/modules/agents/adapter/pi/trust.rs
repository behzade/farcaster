use std::fs;
use std::path::{Path, PathBuf};

const PROJECT_CONFIG_RESOURCES: &[&str] = &[
    "settings.json",
    "project-tools",
    "extensions",
    "skills",
    "prompts",
    "themes",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
];

use crate::projects::trust_store::{canonical, nearest_decision};
use crate::projects::{AppliedTrust, StartupTrust, TrustChoice};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DefaultProjectTrust {
    Always,
    Never,
    #[default]
    Ask,
}

pub(crate) fn startup_trust(project: &Path) -> Result<StartupTrust, String> {
    let environment = trust_environment(project)?;
    if !has_trust_requiring_resources(project, environment.home.as_deref())? {
        return Ok(StartupTrust::Ready);
    }
    startup_trust_with_agent_dir(project, &environment.agent_dir)
}

fn startup_trust_with_agent_dir(project: &Path, agent_dir: &Path) -> Result<StartupTrust, String> {
    if nearest_decision(&agent_dir.join("trust.json"), project)?.is_some() {
        return Ok(StartupTrust::Ready);
    }
    match default_project_trust(&agent_dir.join("settings.json")) {
        DefaultProjectTrust::Always | DefaultProjectTrust::Never => Ok(StartupTrust::Ready),
        DefaultProjectTrust::Ask => Ok(StartupTrust::Prompt),
    }
}

pub(crate) fn apply(project: &Path, choice: TrustChoice) -> Result<AppliedTrust, String> {
    let trust_path = trust_environment(project)?.agent_dir.join("trust.json");
    crate::projects::apply(&trust_path, project, choice)
}

pub(crate) fn saved_decision(project: &Path) -> Result<Option<(PathBuf, bool)>, String> {
    nearest_decision(
        &trust_environment(project)?.agent_dir.join("trust.json"),
        project,
    )
}

fn has_trust_requiring_resources(project: &Path, home: Option<&Path>) -> Result<bool, String> {
    let project = canonical(project)?;
    if PROJECT_CONFIG_RESOURCES
        .iter()
        .any(|entry| project.join(".pi").join(entry).exists())
    {
        return Ok(true);
    }

    let home_skills = home
        .map(canonical)
        .transpose()?
        .map(|home| home.join(".agents/skills"));
    let mut current = Some(project.as_path());
    while let Some(directory) = current {
        let skills = directory.join(".agents/skills");
        if home_skills.as_deref() != Some(skills.as_path()) && skills.exists() {
            return Ok(true);
        }
        current = directory.parent();
    }
    Ok(false)
}

fn default_project_trust(path: &Path) -> DefaultProjectTrust {
    let Ok(bytes) = fs::read(path) else {
        return DefaultProjectTrust::Ask;
    };
    let Ok(settings) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return DefaultProjectTrust::Ask;
    };
    match settings
        .get("defaultProjectTrust")
        .and_then(|value| value.as_str())
    {
        Some("always") => DefaultProjectTrust::Always,
        Some("never") => DefaultProjectTrust::Never,
        Some("ask") | None => DefaultProjectTrust::Ask,
        Some(_) => DefaultProjectTrust::Ask,
    }
}

struct TrustEnvironment {
    agent_dir: PathBuf,
    home: Option<PathBuf>,
}

fn trust_environment(project: &Path) -> Result<TrustEnvironment, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| canonical(&path).ok());
    let root = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".pi/agent")))
        .ok_or_else(|| "HOME is not set and PI_CODING_AGENT_DIR is not set".to_owned())?;
    let agent_dir = if root.is_absolute() {
        root
    } else {
        project.join(root)
    };
    Ok(TrustEnvironment { agent_dir, home })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::trust_store::update_trust_file;
    use tempfile::tempdir;

    #[test]
    fn unresolved_resources_prompt_unless_settings_or_saved_trust_decide()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("project");
        let agent = temp.path().join("agent");
        fs::create_dir_all(project.join(".pi/extensions"))?;
        assert_eq!(
            startup_trust_with_agent_dir(&project, &agent)?,
            StartupTrust::Prompt
        );

        fs::create_dir_all(&agent)?;
        fs::write(
            agent.join("settings.json"),
            r#"{"defaultProjectTrust":"never"}"#,
        )?;
        assert_eq!(
            startup_trust_with_agent_dir(&project, &agent)?,
            StartupTrust::Ready
        );

        fs::write(agent.join("settings.json"), "{}")?;
        update_trust_file(&agent.join("trust.json"), &[(project.clone(), Some(true))])?;
        assert_eq!(
            startup_trust_with_agent_dir(&project, &agent)?,
            StartupTrust::Ready
        );
        Ok(())
    }

    #[test]
    fn user_global_agent_skills_do_not_require_project_trust()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let home = temp.path().join("home");
        let project = home.join("projects/app");
        fs::create_dir_all(home.join(".agents/skills"))?;
        fs::create_dir_all(&project)?;
        assert!(!has_trust_requiring_resources(&project, Some(&home))?);
        Ok(())
    }

    #[test]
    fn project_resources_require_a_decision_and_parent_decisions_are_inherited()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("group/project");
        fs::create_dir_all(project.join(".pi/extensions"))?;
        let agent = temp.path().join("agent");
        assert!(has_trust_requiring_resources(&project, None)?);
        assert_eq!(nearest_decision(&agent.join("trust.json"), &project)?, None);

        update_trust_file(
            &agent.join("trust.json"),
            &[(project.parent().expect("parent").to_path_buf(), Some(true))],
        )?;
        assert_eq!(
            nearest_decision(&agent.join("trust.json"), &project)?,
            Some((project.parent().expect("parent").canonicalize()?, true))
        );
        Ok(())
    }
}
