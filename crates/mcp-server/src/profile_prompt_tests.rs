use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Instant,
};

#[test]
fn worker_admission_and_setup_use_project_executables() -> Result<(), String> {
    use agents::{Backend, HarnessAccessMode, WorkerLaunch, WorkerSession, WorkerSessionFactory};
    use std::os::unix::fs::PermissionsExt as _;

    const CHILD: &str = "FARCASTER_TEST_PROJECT_EXECUTABLE";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                std::thread::current().name().unwrap(),
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("PATH", "/usr/bin:/bin")
            .env_remove("FARCASTER_CODEX_PATH")
            .status()
            .map_err(|error| error.to_string())?;
        assert!(status.success());
        return Ok(());
    }
    struct Factory;
    impl WorkerSessionFactory for Factory {
        fn create(&self, _: WorkerLaunch) -> Result<Box<dyn WorkerSession>, String> {
            Err("admission fixture does not start a backend".into())
        }
    }
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = temp.path().canonicalize().unwrap();
    let bin = project.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join("codex");
    std::fs::write(&executable, "#!/bin/sh\nexit 99\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    agents::set_test_project_environment(&project, vec![("PATH".into(), bin.into_os_string())]);
    assert!(
        !agents::backend_statuses()
            .iter()
            .find(|status| status.id == Backend::Codex)
            .unwrap()
            .available
    );

    let config = agents::AgentLaunchConfig::default();
    let profile_id = uuid::Uuid::new_v4().to_string();
    config.profiles.replace(vec![agents::HarnessProfile {
        id: profile_id.clone(),
        name: "Named Codex".into(),
        backend: Backend::Codex,
        executable: "codex".into(),
        data_directory: None,
    }])?;
    let pool = agents::WorkerPool::new(
        std::collections::BTreeMap::from([(
            Backend::Codex,
            Arc::new(Factory) as Arc<dyn WorkerSessionFactory>,
        )]),
        Backend::Codex,
        project.clone(),
        2,
    )?;
    let parent = agents::CallerRegistry::shared().issue_with_access(
        &project,
        agents::CallerProfile {
            backend: Backend::Codex,
            provider: Some("openai".into()),
            model: Some("model".into()),
            effort: None,
        },
        None,
        HarnessAccessMode::Full,
    );
    parent.bind("project-executable-parent");
    for (name, selector) in [("builtin", None), ("named", Some(profile_id.clone()))] {
        parent.set_harness_profile_id(selector);
        let result = crate::workers::send_configurable(
            &pool,
            crate::workers::SendParams {
                to: Some(name.into()),
                message: "inspect".into(),
                profile: None,
            },
            Some(parent.token().into()),
            &agents::WorkerProfiles::default(),
            |model, profile, project, mode| {
                crate::workers::launch_access_mode(&config, model, profile, project, mode, &[])
            },
            |_, _| panic!("inherit must not configure a preset"),
        )?;
        assert_eq!(result["created"], true, "{name}");
    }
    let model = agents::WorkerExecution {
        harness: Backend::Codex,
        provider: "openai".into(),
        model: "model".into(),
        effort: None,
        service_tier: None,
    };
    for executable_mode in [0o755, 0o644] {
        std::fs::set_permissions(
            &executable,
            std::fs::Permissions::from_mode(executable_mode),
        )
        .unwrap();
        let available = executable_mode == 0o755;
        let backends = available_worker_backends(&config, &project);
        assert_eq!(backends.contains(&Backend::Codex), available);
        assert_eq!(
            fallback_harnesses(&project, HarnessAccessMode::Full, &backends, &[])
                .contains(&Backend::Codex),
            available
        );
        for selector in [None, Some(profile_id.as_str())] {
            assert_eq!(
                crate::workers::launch_access_mode(
                    &config,
                    &model,
                    selector,
                    &project,
                    HarnessAccessMode::Full,
                    &[]
                )
                .is_some(),
                available
            );
        }
    }
    std::fs::remove_file(executable).unwrap();
    assert!(!available_worker_backends(&config, &project).contains(&Backend::Codex));
    assert!(
        crate::workers::launch_access_mode(
            &config,
            &model,
            None,
            &project,
            HarnessAccessMode::Full,
            &[]
        )
        .is_none()
    );
    Ok(())
}

#[test]
fn concurrent_sends_share_one_profile_choice_and_its_result() {
    let model = agents::WorkerExecution {
        harness: agents::Backend::Pi,
        provider: "openai".into(),
        model: "chosen".into(),
        effort: None,
        service_tier: None,
    };
    check_burst("selected", Ok(model));
    check_burst("cancelled", Err("worker creation cancelled".into()));
}

#[test]
fn fallback_offers_only_harnesses_that_can_run_at_the_parent_access_mode() {
    let backends = [agents::Backend::Pi, agents::Backend::Codex];
    let project = std::path::Path::new("/profile-prompt-test");
    let access = agents::delegated_worker_access_mode(
        agents::Backend::Pi,
        agents::HarnessAccessMode::Sandboxed,
    );
    assert_eq!(access, agents::HarnessAccessMode::Auto);
    assert_eq!(
        fallback_harnesses(project, access, &backends, &[]),
        vec![agents::Backend::Codex]
    );
    assert!(fallback_harnesses(project, access, &[agents::Backend::Pi], &[]).is_empty());
    assert_eq!(
        fallback_harnesses(project, agents::HarnessAccessMode::Full, &backends, &[]),
        backends.to_vec()
    );
    let empty_catalog = storage::CachedConfigurationCatalog {
        harness: agents::Backend::Codex,
        profile_id: None,
        project: project.into(),
        catalog: agents::ConfigurationCatalog::default(),
    };
    assert_eq!(
        fallback_harnesses(
            project,
            agents::HarnessAccessMode::Full,
            &backends,
            &[empty_catalog]
        ),
        vec![agents::Backend::Pi]
    );
}

fn check_burst(parent: &str, expected: Selection) {
    let key = (parent.to_owned(), "light".to_owned());
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let (started_tx, started_rx) = mpsc::channel();
    let choices = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::new();
    for _ in 0..20 {
        let key = key.clone();
        let choices = choices.clone();
        let release_rx = release_rx.clone();
        let started_tx = started_tx.clone();
        let expected = expected.clone();
        let handle = thread::spawn(move || {
            select_once(key, || {
                choices.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
                expected
            })
        });
        threads.push(handle);
        // The first caller must be inside its selection before the rest join.
        if threads.len() == 1 {
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while flights()
        .lock()
        .unwrap()
        .get(&key)
        .is_none_or(|flight| Arc::strong_count(flight) < 21)
    {
        assert!(
            Instant::now() < deadline,
            "all sends should join one selection"
        );
        thread::yield_now();
    }
    release_tx.send(()).unwrap();
    for handle in threads {
        assert_eq!(handle.join().unwrap(), expected);
    }
    assert_eq!(choices.load(Ordering::SeqCst), 1);
    assert!(!flights().lock().unwrap().contains_key(&key));
}
