use workgraph::{
    adapter::SqliteAdapter,
    contract::{EditAction, EditRequest, EditResult, IssueStatus},
    core::WorkGraph,
};

use super::persistence::{add_issue_note, create_issue, load_issues, update_issue_status};

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
            project: project_key.clone(),
            idempotency_key: "note".into(),
            action: EditAction::AddNote {
                number: prerequisite.number,
                body: "Ready for the next session".into(),
                expected_version: Some(prerequisite.version),
            },
        })
        .expect("add note");
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

    let loaded = load_issues(database.clone(), project.clone()).expect("board load");
    assert_eq!(loaded.issues.len(), 2);
    assert_eq!(loaded.notes.len(), 1);
    assert_eq!(loaded.notes[0].body, "Ready for the next session");
    assert!(loaded.ready.contains(&prerequisite.number));
    assert!(loaded.blocked.contains(&blocked.number));
    assert_eq!(loaded.next, Some(prerequisite.number));

    let prerequisite_version = loaded
        .issues
        .iter()
        .find(|issue| issue.number == prerequisite.number)
        .expect("loaded prerequisite")
        .version;
    let with_note = add_issue_note(
        database.clone(),
        project.clone(),
        prerequisite.number,
        prerequisite_version,
        "Second durable note".into(),
    )
    .expect("native note update");
    assert_eq!(with_note.notes.len(), 2);
    let updated_version = with_note
        .issues
        .iter()
        .find(|issue| issue.number == prerequisite.number)
        .expect("updated prerequisite")
        .version;
    let updated = update_issue_status(
        database.clone(),
        project.clone(),
        prerequisite.number,
        IssueStatus::InProgress,
        updated_version,
    )
    .expect("native status update");
    assert_eq!(
        updated
            .issues
            .iter()
            .find(|issue| issue.number == prerequisite.number)
            .map(|issue| issue.status),
        Some(IssueStatus::InProgress)
    );

    let (with_created, created_number) = create_issue(
        database,
        project,
        "Created in the native board".into(),
        "Persist the issue without leaving Pi".into(),
    )
    .expect("native issue create");
    assert_eq!(with_created.issues.len(), 3);
    assert_eq!(
        with_created
            .issues
            .iter()
            .find(|issue| issue.number == created_number)
            .map(|issue| issue.title.as_str()),
        Some("Created in the native board")
    );
}
