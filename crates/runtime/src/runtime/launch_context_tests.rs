use super::*;

#[test]
fn native_session_profile_comes_from_storage_and_clears_previous_selection() -> Result<(), String> {
    let host = crate::test_support::host();
    let state = host.state_store()?;
    let project = std::path::PathBuf::from("/launch-context-project");
    let path = project.join("native-pi.jsonl");
    let profile = uuid::Uuid::new_v4().to_string();
    let metadata = crate::agents::SessionMetadata {
        harness: crate::agents::Backend::Pi,
        profile_id: Some(profile.clone()),
        id: "native-pi".into(),
        path: path.clone(),
        project,
        title: None,
        first_user_message: None,
        parent_session: None,
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: None,
        usage: None,
        is_running: false,
    };
    state.with(|store| store.update_session_metadata(&metadata))?;
    let base = AgentLaunchConfig::default();
    let selected = configuration_for_target(&base, Some(&state), LaunchTarget::Session(&path))?;
    assert_eq!(selected.profile_id.as_deref(), Some(profile.as_str()));
    let builtin = configuration_for_target(
        &selected,
        Some(&state),
        LaunchTarget::Session(Path::new("/different-native-session.jsonl")),
    )?;
    assert_eq!(builtin.profile_id, None);
    Ok(())
}
