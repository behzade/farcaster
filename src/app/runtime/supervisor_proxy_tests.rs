use super::*;

#[test]
fn supervisor_proxy_changes_reach_later_worker_launches() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = project.path().join("state.sqlite3");
    let script = project.path().join("fake-pi.sh");
    std::fs::write(&script, include_str!("../../../tests/fixtures/fake-pi.sh"))
        .map_err(|error| error.to_string())?;

    let mut worker_command =
        AgentLaunchConfig::test_script(&script, vec!["worker-launch-config".into()]);
    worker_command.app_proxy = Some("http://stale-proxy.example:8000".into());
    let (factories, backend) = crate::agents::worker_factories(worker_command);
    let pool = crate::agents::WorkerPool::new(factories, backend, project.path().to_owned(), 2)?;
    let (mut supervisor, commands) = test_supervisor(
        project.path().to_owned(),
        StateStore::open_at(&database)?,
        AgentLaunchConfig::test_script(&script, vec!["normal".into()]),
    );

    crate::app::mcp_server::with_test_worker_pool(pool.clone(), || -> Result<(), String> {
        let proxy = "http://127.0.0.1:8118";
        commands
            .send(RuntimeCommand::SetAppProxy(Some(proxy.into())))
            .map_err(|error| error.to_string())?;
        assert!(supervisor.process_next_command());
        assert_eq!(supervisor.process_command.app_proxy.as_deref(), Some(proxy));
        assert_eq!(pool.app_proxy()?.as_deref(), Some(proxy));

        pool.start(worker_request(project.path(), "proxied"))?;
        assert_eq!(
            read_spawned_proxy(project.path())?,
            format!("{proxy}\n{proxy}\n")
        );

        commands
            .send(RuntimeCommand::SetAppProxy(None))
            .map_err(|error| error.to_string())?;
        assert!(supervisor.process_next_command());
        assert_eq!(supervisor.process_command.app_proxy, None);
        assert_eq!(pool.app_proxy()?, None);

        pool.start(worker_request(project.path(), "cleared"))?;
        let cleared = read_spawned_proxy(project.path())?;
        assert!(!cleared.contains("stale-proxy.example"), "{cleared}");
        assert!(!cleared.contains("127.0.0.1:8118"), "{cleared}");
        Ok(())
    })
}

fn test_supervisor(
    project: PathBuf,
    state: StateStore,
    process_command: AgentLaunchConfig,
) -> (Supervisor, mpsc::Sender<RuntimeCommand>) {
    let (commands, command_rx) = mpsc::channel();
    let (events, _) = mpsc::channel();
    let (wake, _) = async_channel::bounded(1);
    let (_, configuration_rx) = mpsc::channel();
    (
        Supervisor {
            process_command,
            command_rx,
            event_tx: UiEventSender { events, wake },
            supervisor_thread: thread::current(),
            catalog_key: "catalog".into(),
            actors: HashMap::new(),
            selected: "selected".into(),
            selected_project: project,
            selected_session: None,
            generation: 0,
            latest: HashMap::new(),
            catalog_sessions: Vec::new(),
            catalog_generation: 0,
            actor_paths: HashMap::new(),
            failed_actor_shutdowns: HashMap::new(),
            interacted: HashSet::new(),
            document_revisions: HashMap::new(),
            pending_extensions: HashMap::new(),
            active_dialogs: HashMap::new(),
            needs_input: HashSet::new(),
            clock: 0,
            last_touch: HashMap::new(),
            configurations: HarnessConfigurationStore::default(),
            catalog_state: Some(state),
            configuration_catalogs: Vec::new(),
            configuration_rx,
            configuration_tx: None,
            configuration_requests: HashSet::new(),
            published_statuses: HashMap::new(),
            recovery: Default::default(),
            published_recovery_selection: None,
        },
        commands,
    )
}

fn worker_request(project: &std::path::Path, name: &str) -> crate::agents::StartWorker {
    crate::agents::StartWorker {
        project: project.to_owned(),
        name: name.into(),
        prompt: "work".into(),
        backend: "pi".into(),
        parent_session: "parent".into(),
        parent_worker_id: None,
        context: crate::agents::WorkerContext::Fresh,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Sandboxed,
    }
}

fn read_spawned_proxy(project: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(project.join("worker-launch-proxy")).map_err(|error| error.to_string())
}
