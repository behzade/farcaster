use workgraph::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, EditResult},
    core::WorkGraph,
};

use super::adapter::load_issues;

#[test]
fn board_loader_reads_issues_from_the_shared_gui_database() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("gui-state.sqlite3");
    let project = directory.path().join("project");
    std::fs::create_dir(&project).expect("project directory");
    let _state = crate::state::StateStore::open_at(&database).expect("GUI state");
    let adapter = SqliteAdapter::open(&database).expect("workgraph adapter");
    let mut graph = WorkGraph::new(adapter);
    let project_key = project
        .canonicalize()
        .expect("canonical project")
        .into_os_string()
        .into_string()
        .expect("UTF-8 project");
    let EditResult::Issue(prerequisite) = graph
        .edit(&EditRequest {
            project: project_key.clone(),
            idempotency_key: "prerequisite".into(),
            action: EditAction::Create {
                title: "Prerequisite".into(),
                body: String::new(),
                priority: 2,
            },
        })
        .expect("create prerequisite")
    else {
        panic!("issue result");
    };
    let EditResult::Issue(blocked) = graph
        .edit(&EditRequest {
            project: project_key.clone(),
            idempotency_key: "blocked".into(),
            action: EditAction::Create {
                title: "Blocked issue".into(),
                body: String::new(),
                priority: 0,
            },
        })
        .expect("create blocked issue")
    else {
        panic!("issue result");
    };
    graph
        .edit(&EditRequest {
            project: project_key,
            idempotency_key: "dependency".into(),
            action: EditAction::AddDependency {
                number: blocked.number,
                depends_on: prerequisite.number,
                expected_version: Some(blocked.version),
            },
        })
        .expect("add dependency");

    let loaded = load_issues(database, project).expect("board load");
    assert_eq!(loaded.issues.len(), 2);
    assert!(loaded.ready.contains(&prerequisite.number));
    assert!(loaded.blocked.contains(&blocked.number));
    assert_eq!(loaded.next, Some(prerequisite.number));
}
