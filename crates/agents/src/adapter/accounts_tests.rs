use super::*;

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
