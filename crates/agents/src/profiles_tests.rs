use super::*;

#[test]
fn profile_data_location_matches_its_backend() {
    let mut profile = HarnessProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Claudex".into(),
        backend: Backend::Claude,
        executable: "/usr/bin/claudex".into(),
        data_directory: Some("/home/user/.claudex".into()),
    };
    assert!(profile.validate().is_ok());
    assert_eq!(profile.data_environment_key(), Some("CLAUDE_CONFIG_DIR"));
    profile.backend = Backend::Codex;
    assert_eq!(profile.data_environment_key(), Some("CODEX_HOME"));
    profile.backend = Backend::Pi;
    assert!(profile.validate().is_err());
}

#[test]
fn command_names_and_profile_locators_are_valid() {
    let id = uuid::Uuid::new_v4().to_string();
    let profile = HarnessProfile {
        id: id.clone(),
        name: "claudex".into(),
        backend: Backend::Claude,
        executable: "claudex".into(),
        data_directory: None,
    };
    assert!(profile.validate().is_ok());
    assert_eq!(
        profile_id_from_locator(&PathBuf::from(format!("/app/profiles/{id}/claude/session"))),
        Some(id)
    );
    assert_eq!(
        profile_id_from_locator(Path::new("/app/claude/session")),
        None
    );
    assert!(
        HarnessProfile {
            executable: "scripts/claudex".into(),
            ..profile
        }
        .validate()
        .is_err()
    );
}
