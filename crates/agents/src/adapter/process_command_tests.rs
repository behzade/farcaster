use super::*;

#[test]
fn command_sets_app_proxy_without_a_captured_environment() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let proxy = "http://127.0.0.1:8118";
    let command = AgentLaunchConfig {
        program: "/usr/bin/true".into(),
        app_proxy: Some(proxy.into()),
        ..AgentLaunchConfig::default()
    }
    .command(project.path())?;
    let environment = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(ToOwned::to_owned)))
        .collect::<Vec<_>>();

    for name in ["http_proxy", "https_proxy"] {
        assert!(environment.iter().any(|(actual, value)| {
            actual == name && value.as_deref() == Some(std::ffi::OsStr::new(proxy))
        }));
    }
    for name in ["no_proxy", "NO_PROXY"] {
        let value = environment
            .iter()
            .find(|(actual, _)| actual == name)
            .and_then(|(_, value)| value.as_ref())
            .expect("local bypass is set");
        assert!(
            value
                .to_string_lossy()
                .split(',')
                .any(|host| host == "127.0.0.1")
        );
    }
    Ok(())
}

#[test]
fn command_uses_profile_executable_and_data_directory() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let profile = crate::HarnessProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "codex2".into(),
        backend: crate::Backend::Codex,
        executable: executable.clone(),
        data_directory: Some(project.path().join("codex2")),
    };
    let profiles = std::sync::Arc::new(crate::HarnessProfiles::default());
    profiles.replace(vec![profile.clone()])?;
    let config = AgentLaunchConfig {
        program: "/missing/base-program".into(),
        profiles,
        profile_id: Some(profile.id.clone()),
        session_locator_root: Some(project.path().join("locators")),
        ..AgentLaunchConfig::default()
    };
    let command = config.command(project.path())?;
    assert_eq!(
        command.get_program(),
        executable
            .canonicalize()
            .map_err(|error| error.to_string())?
    );
    assert!(command.get_envs().any(|(key, value)| key == "CODEX_HOME"
        && value == Some(profile.data_directory.as_ref().unwrap().as_os_str())));
    assert_eq!(
        config.locator_root(),
        Some(project.path().join("locators/profiles").join(&profile.id))
    );
    assert!(
        config
            .validate_profile_backend(crate::Backend::Claude)
            .is_err()
    );
    let mut base = config.clone();
    base.profile_id = None;
    base.prompt_boundary_url = Some("http://parent-hook".into());
    let mut launch = crate::WorkerLaunch {
        harness_profile_id: config.profile_id.clone(),
        slot: None,
        worker_id: "worker".into(),
        worker_name: "worker".into(),
        project: project.path().into(),
        parent_session: "parent".into(),
        parent_worker_id: None,
        context: crate::WorkerContext::Fresh,
        provider: None,
        model: None,
        effort: None,
        service_tier: None,
        access_mode: crate::HarnessAccessMode::Full,
        app_proxy: Some("http://child-proxy".into()),
        ephemeral: false,
    };
    let inherited = base.for_worker(crate::Backend::Codex, &launch)?;
    assert_eq!(inherited.profile_id, config.profile_id);
    assert_eq!(inherited.access_mode, launch.access_mode);
    assert_eq!(inherited.app_proxy, launch.app_proxy);
    assert_eq!(inherited.prompt_boundary_url, None);
    let command = inherited.command(project.path())?;
    assert_eq!(command.get_program(), executable.canonicalize().unwrap());
    assert!(command.get_envs().any(|(key, value)| key == "CODEX_HOME"
        && value == Some(profile.data_directory.as_ref().unwrap().as_os_str())));
    assert!(base.for_worker(crate::Backend::Claude, &launch).is_err());
    launch.harness_profile_id = None;
    let explicit = config.for_worker(crate::Backend::Claude, &launch)?;
    assert_eq!(
        explicit.profile_id, None,
        "explicit presets must not inherit a stale profile"
    );
    Ok(())
}
