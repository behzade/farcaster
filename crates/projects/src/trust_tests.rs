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
