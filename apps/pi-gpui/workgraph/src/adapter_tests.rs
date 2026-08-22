use crate::{
    adapter::SqliteAdapter,
    contract::{CompletionRequirement, EditAction, EditRequest, EditResult, SearchRequest, SearchResult},
    core::WorkGraph,
};

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
                files: vec!["apps/pi-gpui".into()],
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
