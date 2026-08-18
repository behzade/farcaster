use crate::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, EditResult, SearchRequest, SearchResult},
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
            .query_row("SELECT count(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .expect("GUI projects"),
        0
    );
}

#[test]
fn unsupported_future_schema_is_rejected_without_creating_tables() {
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
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name LIKE 'wg_%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("workgraph table count");
    assert_eq!(created, 0);
}

#[test]
fn sqlite_adapter_persists_core_results_across_instances() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let mut graph = WorkGraph::new(SqliteAdapter::open(&path).expect("adapter"));
    let EditResult::Issue(created) = graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "create".into(),
            action: EditAction::Create {
                title: "Persisted".into(),
                body: String::new(),
                priority: 0,
            },
        })
        .expect("create")
    else {
        panic!("issue result");
    };
    drop(graph);

    let mut reopened = WorkGraph::new(SqliteAdapter::open(path).expect("reopen adapter"));
    let result = reopened
        .search(&SearchRequest::Issue {
            project: "/project".into(),
            number: created.number,
        })
        .expect("read issue");
    assert!(matches!(result, SearchResult::Issue(detail) if detail.issue == created));
}
