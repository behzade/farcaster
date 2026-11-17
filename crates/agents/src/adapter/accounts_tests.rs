use super::*;

#[test]
fn environment_sources_share_inherited_credentials_but_separate_project_overrides() {
    let parent = std::env::var_os("PATH").expect("test runner PATH");
    let first = Path::new("/first-project");
    let second = Path::new("/second-project");
    let mut command = std::process::Command::new("unused");
    command.env_clear().env("PATH", parent);
    let inherited = environment_source(&command, "PATH", first).unwrap();
    assert_eq!(
        inherited,
        environment_source(&command, "PATH", second).unwrap()
    );
    command.env("PATH", "fixture-credential-a");
    let local = environment_source(&command, "PATH", first).unwrap();
    command.env("PATH", "fixture-credential-b");
    let other = environment_source(&command, "PATH", second).unwrap();
    assert_ne!(local, other);
    assert_ne!(local, inherited);
    // Rotating an override keeps its connection origin stable.
    assert_eq!(local, environment_source(&command, "PATH", first).unwrap());
    assert!(!local.contains("fixture-credential"));
    assert!(!other.contains("fixture-credential"));
    command.env_remove("PATH");
    assert!(environment_source(&command, "PATH", first).is_none());
}

#[test]
fn discovery_identifies_the_store_even_when_missing_or_empty() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    let config = AgentLaunchConfig::default();
    let missing = stored_connections(&config, Backend::Pi, Path::new("/project"), &path).unwrap();
    assert!(missing.observations.is_empty());
    std::fs::write(&path, r#"{"openai":{"type":"oauth","access":"fake"}}"#).unwrap();
    let configured =
        stored_connections(&config, Backend::Pi, Path::new("/other-project"), &path).unwrap();
    assert_eq!(configured.observations.len(), 1);
    assert!(missing.source.same_store(&configured.source));
    assert!(
        configured
            .source
            .same_store(&configured.observations[0].source)
    );
    std::fs::write(&path, "{}").unwrap();
    let empty =
        stored_connections(&config, Backend::Pi, Path::new("/third-project"), &path).unwrap();
    assert!(empty.observations.is_empty());
    assert!(empty.source.same_store(&configured.source));
}
