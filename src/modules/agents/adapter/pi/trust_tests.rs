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
fn user_global_agent_skills_do_not_require_project_trust() -> Result<(), Box<dyn std::error::Error>>
{
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
