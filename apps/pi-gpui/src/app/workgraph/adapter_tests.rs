use super::persistence::{add_node, create_plan, link_session, load_plan};

#[test]
fn native_plan_adapter_round_trips_nodes_walk_and_session() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("gui-state.sqlite3");
    let project = directory.path().join("project");
    std::fs::create_dir(&project).expect("project directory");
    let _state = crate::state::StateStore::open_at(&database).expect("GUI state");

    let (created, root) = create_plan(
        database.clone(),
        project.clone(),
        "Git and jj integration".into(),
        "Current product".into(),
    )
    .expect("create plan");
    let snapshot = created.snapshot.expect("snapshot");
    assert_eq!(snapshot.nodes.len(), 1);
    assert_eq!(snapshot.plan.root_node, root);
    let walk = snapshot.walk.expect("default walk");

    let (with_node, node) = add_node(
        database.clone(),
        project.clone(),
        snapshot.plan.number,
        "Both backends expose repository state".into(),
        vec!["apps/pi-gpui/src/vcs".into()],
        Some(root),
        None,
    )
    .expect("add node");
    assert_eq!(
        with_node.snapshot.as_ref().map(|plan| plan.nodes.len()),
        Some(2)
    );
    assert!(
        with_node
            .snapshot
            .as_ref()
            .expect("snapshot")
            .edges
            .iter()
            .any(|edge| edge.from == root && edge.to == node)
    );

    let linked = link_session(
        database.clone(),
        project.clone(),
        walk.number,
        "session-1".into(),
        "/sessions/one.jsonl".into(),
    )
    .expect("link session");
    assert_eq!(
        linked.session_link.as_ref().map(|link| link.walk_number),
        Some(walk.number)
    );

    let loaded = load_plan(database, project, Some("session-1")).expect("load linked plan");
    assert_eq!(
        loaded.snapshot.as_ref().map(|plan| plan.plan.number),
        Some(snapshot.plan.number)
    );
    assert_eq!(
        loaded.snapshot.as_ref().map(|plan| plan.nodes.len()),
        Some(2)
    );
}
