use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupTrust {
    Ready,
    Prompt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrustChoice {
    TrustProject,
    TrustParent,
    DistrustProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustOption {
    pub label: String,
    pub choice: TrustChoice,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppliedTrust {
    pub trusted: bool,
    pub saved_path: Option<PathBuf>,
}

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

pub(crate) fn apply(project: &Path, choice: TrustChoice) -> Result<AppliedTrust, String> {
    let project = canonical(project)?;
    let trust_path = trust_environment(&project)?.agent_dir.join("trust.json");
    let (trusted, saved_path, updates) = match choice {
        TrustChoice::TrustProject => (true, Some(project.clone()), vec![(project, Some(true))]),
        TrustChoice::TrustParent => {
            let parent = project
                .parent()
                .ok_or_else(|| "the project has no parent folder to trust".to_owned())?
                .to_path_buf();
            (
                true,
                Some(parent.clone()),
                vec![(parent, Some(true)), (project, None)],
            )
        }
        TrustChoice::DistrustProject => {
            (false, Some(project.clone()), vec![(project, Some(false))])
        }
    };
    if !updates.is_empty() {
        update_trust_file(&trust_path, &updates)?;
    }
    Ok(AppliedTrust {
        trusted,
        saved_path,
    })
}

pub(crate) fn saved_decision(project: &Path) -> Result<Option<(PathBuf, bool)>, String> {
    nearest_decision(
        &trust_environment(project)?.agent_dir.join("trust.json"),
        project,
    )
}

pub(crate) fn repository_execution_allowed(project: &Path) -> Result<bool, String> {
    let environment = trust_environment(project)?;
    repository_execution_allowed_with_agent_dir(
        project,
        &environment.agent_dir,
        environment.home.as_deref(),
    )
}

fn repository_execution_allowed_with_agent_dir(
    project: &Path,
    agent_dir: &Path,
    home: Option<&Path>,
) -> Result<bool, String> {
    if let Some((_, trusted)) = nearest_decision(&agent_dir.join("trust.json"), project)? {
        return Ok(trusted);
    }
    match default_project_trust(&agent_dir.join("settings.json")) {
        DefaultProjectTrust::Always => Ok(true),
        DefaultProjectTrust::Never => Ok(false),
        DefaultProjectTrust::Ask => Ok(!has_trust_requiring_resources(project, home)?),
    }
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

fn nearest_decision(path: &Path, project: &Path) -> Result<Option<(PathBuf, bool)>, String> {
    let _lock = TrustFileLock::acquire(path)?;
    let data = read_trust_file(path)?;
    let project = canonical(project)?;
    let mut current = Some(project.as_path());
    while let Some(directory) = current {
        let key = directory.display().to_string();
        if let Some(Some(decision)) = data.get(&key) {
            return Ok(Some((directory.to_path_buf(), *decision)));
        }
        current = directory.parent();
    }
    Ok(None)
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

fn update_trust_file(path: &Path, updates: &[(PathBuf, Option<bool>)]) -> Result<(), String> {
    let _lock = TrustFileLock::acquire(path)?;
    let mut data = read_trust_file(path)?;
    for (path, decision) in updates {
        let key = canonical(path)?.display().to_string();
        if let Some(decision) = decision {
            data.insert(key, Some(*decision));
        } else {
            data.remove(&key);
        }
    }
    write_trust_file(path, &data)
}

fn read_trust_file(path: &Path) -> Result<BTreeMap<String, Option<bool>>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("read trust store {}: {error}", path.display())),
    };
    let value = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("decode trust store {}: {error}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("invalid trust store {}: expected an object", path.display()))?;
    let mut data = BTreeMap::new();
    for (key, value) in object {
        let decision = match value {
            serde_json::Value::Bool(decision) => Some(*decision),
            serde_json::Value::Null => None,
            _ => {
                return Err(format!(
                    "invalid trust store {}: value for {key:?} must be true, false, or null",
                    path.display()
                ));
            }
        };
        data.insert(key.clone(), decision);
    }
    Ok(data)
}

fn write_trust_file(path: &Path, data: &BTreeMap<String, Option<bool>>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("trust store has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?;
    let mut bytes = serde_json::to_vec_pretty(data)
        .map_err(|error| format!("encode trust store {}: {error}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
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

fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))
}

struct TrustFileLock {
    path: PathBuf,
}

impl TrustFileLock {
    fn acquire(trust_path: &Path) -> Result<Self, String> {
        let parent = trust_path
            .parent()
            .ok_or_else(|| format!("trust store has no parent: {}", trust_path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
        let path = trust_path.with_extension("json.lock");
        for attempt in 0..10 {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 9 => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age >= Duration::from_secs(10));
                    if stale {
                        let _ = fs::remove_dir_all(&path);
                    } else {
                        thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(error) => return Err(format!("lock trust store {}: {error}", path.display())),
            }
        }
        Err(format!("lock trust store {}: timed out", path.display()))
    }
}

impl Drop for TrustFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn repository_commands_follow_resource_saved_and_default_trust()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("project");
        let agent = temp.path().join("agent");
        fs::create_dir_all(&project)?;
        fs::create_dir_all(&agent)?;

        assert!(repository_execution_allowed_with_agent_dir(
            &project, &agent, None
        )?);
        fs::create_dir_all(project.join(".pi/extensions"))?;
        assert!(!repository_execution_allowed_with_agent_dir(
            &project, &agent, None
        )?);
        fs::write(
            agent.join("settings.json"),
            r#"{"defaultProjectTrust":"never"}"#,
        )?;
        assert!(!repository_execution_allowed_with_agent_dir(
            &project, &agent, None
        )?);
        update_trust_file(&agent.join("trust.json"), &[(project.clone(), Some(true))])?;
        assert!(repository_execution_allowed_with_agent_dir(
            &project, &agent, None
        )?);
        update_trust_file(&agent.join("trust.json"), &[(project.clone(), Some(false))])?;
        assert!(!repository_execution_allowed_with_agent_dir(
            &project, &agent, None
        )?);
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

    #[test]
    fn parent_trust_removes_a_more_specific_project_decision()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let project = temp.path().join("group/project");
        fs::create_dir_all(&project)?;
        let path = temp.path().join("trust.json");
        update_trust_file(&path, &[(project.clone(), Some(false))])?;
        update_trust_file(
            &path,
            &[
                (project.parent().expect("parent").to_path_buf(), Some(true)),
                (project.clone(), None),
            ],
        )?;
        assert_eq!(
            nearest_decision(&path, &project)?,
            Some((project.parent().expect("parent").canonicalize()?, true))
        );
        Ok(())
    }

    #[test]
    fn invalid_trust_values_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let path = temp.path().join("trust.json");
        fs::write(&path, r#"{"/project":"yes"}"#)?;
        assert!(read_trust_file(&path).is_err());
        Ok(())
    }
}
