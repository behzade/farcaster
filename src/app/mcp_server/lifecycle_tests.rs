use super::*;

#[test]
fn disabled_server_leaves_the_port_free_and_can_be_reenabled() {
    let project = tempfile::tempdir().expect("project");
    let (factories, backend) =
        crate::agents::worker_factories(crate::agents::AgentLaunchConfig::default());
    let workers = crate::agents::WorkerPool::new(factories, backend, project.path().to_owned(), 1)
        .expect("workers");
    let (updates, _) = async_channel::bounded(1);
    let service = FarcasterMcp::new(
        project.path().join("state.db"),
        workers,
        updates,
        notices::NoticeBoard::default(),
    );
    let occupied = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let address = occupied.local_addr().expect("address").to_string();
    assert!(ServerState::new(service.clone(), true, &address).is_err());
    let mut server =
        ServerState::new(service, false, &address).expect("disabled startup ignores occupied port");
    server.disable();
    assert!(server.enable(&address).is_err());
    assert!(server.running.is_none());
    drop(occupied);
    for _ in 0..2 {
        server.enable(&address).expect("enable server");
        assert!(
            TcpListener::bind(&address).is_err(),
            "enabled server owns the port"
        );
        server.disable();
        let probe = TcpListener::bind(&address).expect("disabled server releases port");
        drop(probe);
    }
}
