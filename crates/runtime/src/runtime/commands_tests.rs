use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn historical_rename_uses_target_profile_and_not_receiving_actor() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = |name: &str| -> Result<PathBuf, String> {
        let path = project.path().join(name);
        let fixture = include_str!("../../../../tests/fixtures/fake-pi.sh").replace(
            "while IFS= read -r line; do",
            "while IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> \"$0.requests\"",
        );
        std::fs::write(&path, format!("#!/bin/sh\nset -- normal \"$@\"\n{fixture}"))
            .map_err(|error| error.to_string())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        Ok(path)
    };
    let default_executable = executable("default-pi")?;
    let target_executable = executable("target-pi")?;
    let actor_executable = executable("actor-pi")?;
    let profile_id = uuid::Uuid::new_v4().to_string();
    let actor_profile_id = uuid::Uuid::new_v4().to_string();
    let (mut owner, events) = crate::runtime::tests::owner_without_process(project.path().into());
    owner.process_command = AgentLaunchConfig {
        program: default_executable.clone(),
        profile_id: Some(actor_profile_id.clone()),
        ..Default::default()
    };
    owner.process_command.profiles.replace(vec![
        agents::HarnessProfile {
            id: profile_id.clone(),
            name: "Target Pi".into(),
            backend: Backend::Pi,
            executable: target_executable.clone(),
            data_directory: None,
        },
        agents::HarnessProfile {
            id: actor_profile_id.clone(),
            name: "Actor Pi".into(),
            backend: Backend::Pi,
            executable: actor_executable.clone(),
            data_directory: None,
        },
    ])?;
    owner.state = Some(StateStore::open_at(&project.path().join("state.sqlite3"))?.into());

    for (session_id, selected_profile, selected_executable) in [
        ("named", Some(profile_id.clone()), &target_executable),
        ("builtin", None, &default_executable),
    ] {
        let path = project.path().join(format!("{session_id}.jsonl"));
        std::fs::write(
            &path,
            json!({"type": "session", "id": session_id}).to_string(),
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(agents::profile_id_from_locator(&path), None);
        let metadata = agents::SessionMetadata {
            harness: Backend::Pi,
            profile_id: selected_profile.clone(),
            id: session_id.into(),
            path: path.clone(),
            project: project.path().into(),
            title: Some("Old name".into()),
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
        owner
            .state
            .as_ref()
            .expect("state")
            .with(|store| store.update_session_metadata(&metadata).map(|_| ()))?;
        owner.apply_command(RuntimeCommand::RenameSession {
            path: path.clone(),
            harness: Backend::Pi,
            session_id: session_id.into(),
            project: project.path().into(),
            name: "New name".into(),
        });
        match events.try_recv().map_err(|error| error.to_string())? {
            RuntimeEvent::SessionUpdated(session) => {
                assert_eq!(session.title, "New name");
                assert_eq!(session.profile_id, selected_profile);
            }
            RuntimeEvent::SessionsFailed { message, .. } => return Err(message),
            _ => panic!("expected session update"),
        }
        let requests = std::fs::read_to_string(selected_executable.with_extension("requests"))
            .map_err(|error| error.to_string())?;
        assert!(
            requests.contains("\"type\":\"set_session_name\""),
            "{requests}"
        );
        assert!(requests.contains("\"name\":\"New name\""), "{requests}");
        assert_eq!(
            owner.process_command.profile_id,
            Some(actor_profile_id.clone())
        );
        assert_eq!(
            owner
                .state
                .as_ref()
                .expect("state")
                .with(|store| store.session_profile_id(&path))?,
            selected_profile
        );
    }
    assert!(!actor_executable.with_extension("requests").exists());
    Ok(())
}
