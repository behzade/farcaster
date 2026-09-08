use crate::{
    adapter::SqliteAdapter,
    contract::{CompletionRequirement, EditAction, EditRequest, EditResult, SearchRequest, SearchResult},
    core::WorkGraph,
};

#[test]
fn legacy_nodes_without_acceptance_remain_readable() {
    let node: crate::contract::Node = serde_json::from_value(serde_json::json!({
        "planNumber": 1,
        "number": 2,
        "title": "Legacy node",
        "files": [],
        "completion": "revision_or_observation",
        "version": 1,
        "createdAt": 0,
        "updatedAt": 0
    }))
    .expect("legacy node");

    assert!(node.acceptance.is_empty());
}

#[test]
fn legacy_walk_completion_is_materialized_as_global_task_completion() {
    let node: crate::contract::Node = serde_json::from_value(serde_json::json!({
        "planNumber": 1,
        "number": 2,
        "title": "Legacy node",
        "files": [],
        "completion": "revision_or_observation",
        "version": 1,
        "createdAt": 0,
        "updatedAt": 0
    }))
    .expect("legacy node");
    let graph = crate::contract::ProjectGraph {
        nodes: vec![node],
        steps: vec![crate::contract::WalkStep {
            id: 1,
            walk_number: 1,
            node_number: 2,
            parent_step: None,
            outcome: crate::contract::Outcome {
                note: "Legacy done".into(),
                evidence: crate::contract::Evidence {
                    kind: crate::contract::EvidenceKind::Observation,
                    reference: "legacy evidence".into(),
                },
            },
            completed_at: 10,
        }],
        ..crate::contract::ProjectGraph::default()
    };

    let state = graph.task_state(2).expect("legacy task state");
    assert!(state.owner.is_none());
    assert_eq!(
        state
            .completion
            .as_ref()
            .map(|completion| completion.session_id.as_str()),
        Some("legacy")
    );
}

#[test]
fn sqlite_schema_coexists_with_gui_state_tables() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let connection = rusqlite::Connection::open(&path).expect("database");
    connection
        .execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES('schema_version', '4');
             CREATE TABLE projects(path TEXT PRIMARY KEY, added_ms INTEGER NOT NULL);",
        )
        .expect("GUI schema fixture");
    drop(connection);

    let _adapter = SqliteAdapter::open(&path).expect("SQLite adapter");
    let connection = rusqlite::Connection::open(path).expect("database");
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, String>(0)
            )
            .expect("GUI schema version"),
        "4"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM projects", [], |row| row.get::<_, i64>(0))
            .expect("GUI projects"),
        0
    );
}

#[test]
fn unsupported_future_schema_is_rejected_without_creating_plan_tables() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let connection = rusqlite::Connection::open(&path).expect("database");
    connection
        .execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES('schema_version', '4');
             INSERT INTO meta VALUES('workgraph_schema_version', '99');",
        )
        .expect("future schema fixture");
    drop(connection);

    assert!(SqliteAdapter::open(&path).is_err());
    let connection = rusqlite::Connection::open(path).expect("database");
    let created = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('wg_plan_store', 'wg_plan_receipts')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("plan table count");
    assert_eq!(created, 0);
}

#[test]
fn version_two_upgrade_retains_legacy_rows_without_reinterpreting_them() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let connection = rusqlite::Connection::open(&path).expect("database");
    connection
        .execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta VALUES('workgraph_schema_version', '2');
             CREATE TABLE wg_issues(project_id INTEGER, number INTEGER, title TEXT);
             INSERT INTO wg_issues VALUES(1, 1, 'Legacy issue');",
        )
        .expect("version two fixture");
    drop(connection);

    let mut graph = WorkGraph::new(SqliteAdapter::open(&path).expect("upgrade"));
    let result = graph
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("new project graph");
    assert!(matches!(result, SearchResult::Project(graph) if graph.plans.is_empty()));
    drop(graph);

    let connection = rusqlite::Connection::open(path).expect("database");
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM wg_issues", [], |row| row.get::<_, i64>(0))
            .expect("legacy rows"),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM meta WHERE key='workgraph_schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version"),
        "3"
    );
}

#[test]
fn concurrent_sqlite_claim_has_exactly_one_owner() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("claims.sqlite3");
    let mut setup = WorkGraph::new(SqliteAdapter::open(&path).expect("setup adapter"));
    let EditResult::Tasks(snapshot) = setup
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "create-task".into(),
            action: EditAction::CreateTasks {
                nodes: vec![crate::contract::NodeDraft {
                    title: "Exclusive task".into(),
                    acceptance: "One owner".into(),
                }],
                after: None,
                before: None,
            },
        })
        .expect("create task")
    else {
        panic!("tasks result");
    };
    let task = snapshot.plan.root_node;
    drop(setup);

    let adapters = [
        SqliteAdapter::open(&path).expect("first adapter"),
        SqliteAdapter::open(&path).expect("second adapter"),
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles = adapters.into_iter().enumerate().map(|(index, adapter)| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let mut graph = WorkGraph::new(adapter);
            barrier.wait();
            graph.edit(&EditRequest {
                project: "/project".into(),
                idempotency_key: format!("claim-{index}"),
                action: EditAction::ClaimTask {
                    task,
                    session_id: format!("session-{index}"),
                    session_path: format!("/sessions/{index}.jsonl"),
                },
            })
        })
    }).collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread"))
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(crate::core::WorkGraphError::TaskClaimed)))
            .count(),
        1
    );

    let mut reopened = WorkGraph::new(SqliteAdapter::open(path).expect("reopen adapter"));
    let SearchResult::Project(project) = reopened
        .search(&SearchRequest::Project {
            project: "/project".into(),
        })
        .expect("project")
    else {
        panic!("project result");
    };
    assert!(
        project
            .task_state(task)
            .is_some_and(|state| state.owner.is_some())
    );
    assert_eq!(
        project
            .tasks
            .iter()
            .filter(|state| state.owner.is_some())
            .count(),
        1
    );
}

#[test]
fn sqlite_adapter_persists_plan_walk_and_session_across_instances() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let mut graph = WorkGraph::new(SqliteAdapter::open(&path).expect("adapter"));
    let EditResult::Plan(snapshot) = graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "create-plan".into(),
            action: EditAction::CreatePlan {
                title: "VCS integration".into(),
                root_title: "Current product".into(),
                files: vec!["apps/farcaster".into()],
                completion: CompletionRequirement::RevisionOrObservation,
            },
        })
        .expect("create plan")
    else {
        panic!("plan result");
    };
    let walk = snapshot.walk.expect("default walk");
    graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "link".into(),
            action: EditAction::LinkSession {
                walk: walk.number,
                session_id: "session-1".into(),
                session_path: "/sessions/one.jsonl".into(),
            },
        })
        .expect("link session");
    drop(graph);

    let mut reopened = WorkGraph::new(SqliteAdapter::open(path).expect("reopen adapter"));
    let result = reopened
        .search(&SearchRequest::Plan {
            project: "/project".into(),
            plan: snapshot.plan.number,
            walk: Some(walk.number),
        })
        .expect("read plan");
    assert!(matches!(result, SearchResult::Plan(plan) if plan.nodes.len() == 1 && plan.sessions.len() == 1));
}
