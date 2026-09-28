use super::*;
use crate::{SessionLink, TaskCompletion, TaskOwner, TaskState};

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("open database");
    crate::SqliteAdapter::initialize_connection(&connection).expect("initialize graph");
    connection
}

fn store(connection: &Connection, graph: ProjectGraph) {
    let stored = StoredProject {
        graph,
        ..StoredProject::new()
    };
    connection
        .execute(
            "INSERT INTO wg_plan_store VALUES('/project', ?1, 1)",
            [serde_json::to_string(&stored).expect("encode graph")],
        )
        .expect("store graph");
}

fn load(connection: &Connection) -> StoredProject {
    let json: String = connection
        .query_row("SELECT data_json FROM wg_plan_store", [], |row| row.get(0))
        .expect("read graph");
    serde_json::from_str(&json).expect("decode graph")
}

fn owner_task(task: u64, id: &str, path: &str) -> TaskState {
    TaskState {
        plan_number: 1,
        task,
        owner: Some(TaskOwner {
            session_id: id.into(),
            session_path: path.into(),
            claimed_at: 1,
        }),
        completion: None,
    }
}

#[test]
fn remaps_owners_without_links_and_keeps_ambiguous_references() {
    let connection = database();
    store(
        &connection,
        ProjectGraph {
            tasks: vec![
                owner_task(1, "native", "/known"),
                owner_task(2, "native", "/unknown"),
            ],
            ..Default::default()
        },
    );
    remap_session_keys(&connection, |project, id, path| {
        assert_eq!(project, "/project");
        (id == "native" && path == Some("/known")).then(|| "app-session:7".into())
    })
    .expect("remap");
    let stored = load(&connection);
    assert_eq!(
        stored.graph.tasks[0]
            .owner
            .as_ref()
            .expect("owner")
            .session_id,
        "app-session:7"
    );
    assert_eq!(
        stored.graph.tasks[1]
            .owner
            .as_ref()
            .expect("owner")
            .session_id,
        "native"
    );
}

#[test]
fn merge_coalesces_links_and_preserves_all_owners_and_completions() {
    let connection = database();
    let mut completed = owner_task(3, "old", "/old");
    completed.owner = None;
    completed.completion = Some(TaskCompletion {
        session_id: "old".into(),
        outcome: crate::Outcome {
            note: "done".into(),
            evidence: crate::Evidence {
                kind: crate::EvidenceKind::Observation,
                reference: "checked".into(),
            },
        },
        completed_at: 3,
    });
    store(
        &connection,
        ProjectGraph {
            sessions: vec![
                SessionLink {
                    session_id: "old".into(),
                    session_path: "/old".into(),
                    plan_number: 1,
                    walk_number: 1,
                    linked_at: 1,
                },
                SessionLink {
                    session_id: "keep".into(),
                    session_path: "/keep".into(),
                    plan_number: 1,
                    walk_number: 2,
                    linked_at: 2,
                },
            ],
            tasks: vec![
                owner_task(1, "old", "/old"),
                owner_task(2, "keep", "/keep"),
                completed,
            ],
            ..Default::default()
        },
    );
    remap_session_keys(&connection, |_, id, _| (id == "old").then(|| "keep".into()))
        .expect("merge");
    let stored = load(&connection);
    assert_eq!(stored.graph.sessions.len(), 1);
    assert_eq!(stored.graph.sessions[0].walk_number, 2);
    assert!(
        stored.graph.tasks[..2]
            .iter()
            .all(|task| task.owner.as_ref().expect("owner").session_id == "keep")
    );
    assert_eq!(
        stored.graph.tasks[2]
            .completion
            .as_ref()
            .expect("completion")
            .session_id,
        "keep"
    );
}

#[test]
fn remapping_uses_the_outer_transaction_and_tolerates_uninitialized_graphs() {
    let mut connection = Connection::open_in_memory().expect("open database");
    remap_session_keys(&connection, |_, _, _| panic!("no graph yet")).expect("no graph");
    crate::SqliteAdapter::initialize_connection(&connection).expect("initialize graph");
    store(
        &connection,
        ProjectGraph {
            tasks: vec![owner_task(1, "old", "/old")],
            ..Default::default()
        },
    );
    {
        let tx = connection.transaction().expect("begin outer transaction");
        remap_session_keys(&tx, |_, _, _| Some("new".into())).expect("remap in transaction");
        assert_eq!(
            load(&tx).graph.tasks[0]
                .owner
                .as_ref()
                .expect("owner")
                .session_id,
            "new"
        );
        // Dropping the caller's transaction rolls back the graph change too.
    }
    assert_eq!(
        load(&connection).graph.tasks[0]
            .owner
            .as_ref()
            .expect("owner")
            .session_id,
        "old"
    );
}

#[test]
fn merged_claims_remain_completable_without_leaving_discarded_walks_active() {
    use crate::{
        EditAction, EditRequest, Evidence, EvidenceKind, NodeDraft, Outcome, SqliteAdapter,
        WorkGraph,
    };

    for complete in [false, true] {
        let mut connection = database();
        let edit = |connection: &mut Connection, id: &str, action| {
            WorkGraph::new(SqliteAdapter::borrow(connection))
                .edit(&EditRequest {
                    project: "/project".into(),
                    idempotency_key: id.into(),
                    action,
                })
                .expect("edit graph")
        };
        for task in 1..=2 {
            edit(
                &mut connection,
                &format!("create-{task}"),
                EditAction::CreateTasks {
                    nodes: vec![NodeDraft {
                        title: format!("Task {task}"),
                        acceptance: "checked".into(),
                    }],
                    after: None,
                    before: None,
                },
            );
            edit(
                &mut connection,
                &format!("claim-{task}"),
                EditAction::ClaimTask {
                    task,
                    session_id: format!("session-{task}"),
                    session_path: format!("/session-{task}"),
                },
            );
        }
        let tx = connection.transaction().expect("begin merge");
        remap_session_keys(&tx, |_, key, _| {
            (key == "session-1").then(|| "session-2".into())
        })
        .expect("merge identities");
        tx.commit().expect("commit merge");
        let graph = load(&connection).graph;
        assert_eq!(graph.sessions.len(), 1);
        let selected = graph.sessions[0].walk_number;
        let discarded = graph
            .walks
            .iter()
            .find(|walk| walk.number != selected)
            .expect("discarded walk");
        assert_eq!(discarded.current_node, None);
        assert!(
            graph.tasks.iter().all(
                |task| task.owner.as_ref().expect("claim preserved").session_id == "session-2"
            )
        );
        for task in 1..=2 {
            let action = if complete {
                EditAction::CompleteTask {
                    task,
                    session_id: "session-2".into(),
                    outcome: Outcome {
                        note: "checked".into(),
                        evidence: Evidence {
                            kind: EvidenceKind::Observation,
                            reference: "checked".into(),
                        },
                    },
                }
            } else {
                EditAction::ReleaseTask {
                    task,
                    session_id: "session-2".into(),
                }
            };
            edit(&mut connection, &format!("finish-{task}"), action);
        }
        let graph = load(&connection).graph;
        assert!(graph.tasks.iter().all(|task| task.owner.is_none()));
        assert!(graph.walks.iter().all(|walk| walk.current_node.is_none()));
        assert!(
            graph
                .tasks
                .iter()
                .all(|task| task.completion.is_some() == complete)
        );
    }
}
