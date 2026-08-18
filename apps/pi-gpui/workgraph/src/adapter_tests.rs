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
fn version_one_database_upgrades_without_losing_issues() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let mut graph = WorkGraph::new(SqliteAdapter::open(&path).expect("adapter"));
    let _created = graph
        .edit(&EditRequest {
            project: "/project".into(),
            idempotency_key: "upgrade-create".into(),
            action: EditAction::Create {
                title: "Keep me".into(),
                body: String::new(),
                priority: 0,
            },
        })
        .expect("create");
    drop(graph);
    let connection = rusqlite::Connection::open(&path).expect("database");
    connection
        .execute("UPDATE meta SET value='1' WHERE key='workgraph_schema_version'", [])
        .expect("downgrade marker");
    connection
        .execute("DROP TABLE wg_session_links", [])
        .expect("version one shape");
    drop(connection);

    let mut reopened = WorkGraph::new(SqliteAdapter::open(&path).expect("upgrade"));
    let result = reopened
        .search(&SearchRequest::Graph {
            project: "/project".into(),
        })
        .expect("graph");
    assert!(matches!(result, SearchResult::Graph(graph) if graph.issues.len() == 1));
}

#[test]
fn session_link_moves_between_issues_and_appears_in_details() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("gui-state.sqlite3");
    let mut graph = WorkGraph::new(SqliteAdapter::open(path).expect("adapter"));
    let mut issues = Vec::new();
    for (key, title) in [("first", "First"), ("second", "Second")] {
        let EditResult::Issue(issue) = graph
            .edit(&EditRequest {
                project: "/project".into(),
                idempotency_key: key.into(),
                action: EditAction::Create {
                    title: title.into(),
                    body: String::new(),
                    priority: 0,
                },
            })
            .expect("create")
        else {
            panic!("issue result");
        };
        issues.push(issue);
    }
    for (key, issue) in [("link-one", &issues[0]), ("link-two", &issues[1])] {
        graph
            .edit(&EditRequest {
                project: "/project".into(),
                idempotency_key: key.into(),
                action: EditAction::LinkSession {
                    number: issue.number,
                    session_id: "session-1".into(),
                    session_path: "/sessions/one.jsonl".into(),
                    expected_version: Some(issue.version),
                },
            })
            .expect("link");
    }
    let detail = graph
        .search(&SearchRequest::Issue {
            project: "/project".into(),
            number: issues[1].number,
        })
        .expect("detail");
    assert!(matches!(detail, SearchResult::Issue(detail) if detail.sessions.len() == 1));
    let first = graph
        .search(&SearchRequest::Issue {
            project: "/project".into(),
            number: issues[0].number,
        })
        .expect("detail");
    assert!(matches!(first, SearchResult::Issue(detail) if detail.sessions.is_empty()));
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
