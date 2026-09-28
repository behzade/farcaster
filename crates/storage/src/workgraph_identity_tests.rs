use super::*;
use crate::agents::Backend;
use workgraph::{ProjectGraph, SessionLink, StoredProject, TaskOwner, TaskState};

fn session(store: &StateStore, project: &str, native: &str, path: &str, harness: Backend) -> i64 {
    store
        .connection
        .execute(
            "INSERT INTO projects(path,added_ms) VALUES(?1,0) ON CONFLICT DO NOTHING",
            [project],
        )
        .expect("insert project");
    store.connection.execute(
        "INSERT INTO sessions(project_id,harness,backend_id,locator,modified_ms,created_ms) VALUES((SELECT id FROM projects WHERE path=?1),?2,?3,?4,0,0)",
        params![project, harness.as_str(), native, path],
    ).expect("insert session");
    store.connection.last_insert_rowid()
}

fn task(number: u64, id: &str, path: &str) -> TaskState {
    TaskState {
        task: number,
        plan_number: 1,
        owner: Some(TaskOwner {
            session_id: id.into(),
            session_path: path.into(),
            claimed_at: 1,
        }),
        completion: None,
    }
}

fn graph(store: &StateStore, project: &str, graph: ProjectGraph) {
    workgraph::SqliteAdapter::initialize_connection(&store.connection).expect("initialize graph");
    let stored = StoredProject {
        graph,
        ..StoredProject::new()
    };
    store
        .connection
        .execute(
            "INSERT INTO wg_plan_store VALUES(?1,?2,1)",
            params![
                project,
                serde_json::to_string(&stored).expect("encode graph")
            ],
        )
        .expect("store graph");
}

fn load(connection: &Connection) -> ProjectGraph {
    let json: String = connection
        .query_row("SELECT data_json FROM wg_plan_store", [], |row| row.get(0))
        .expect("read graph");
    serde_json::from_str::<StoredProject>(&json)
        .expect("decode graph")
        .graph
}

fn legacy(store: &StateStore) {
    store
        .connection
        .execute("DELETE FROM meta WHERE key=?1", [MIGRATION_KEY])
        .expect("restore legacy migration state");
}

#[test]
fn migrates_exact_legacy_paths_without_claiming_ambiguous_owners() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&path)?;
    let base = session(&store, "/project", "same", "/base", Backend::Codex);
    let profile = session(&store, "/project", "same", "/profile", Backend::Codex);
    session(&store, "/project", "ambiguous", "/shared", Backend::Codex);
    session(&store, "/project", "ambiguous", "/shared", Backend::Claude);
    graph(
        &store,
        "/project",
        ProjectGraph {
            sessions: vec![SessionLink {
                session_id: "same".into(),
                session_path: "/profile".into(),
                plan_number: 1,
                walk_number: 1,
                linked_at: 1,
            }],
            tasks: vec![
                task(1, "same", "/base"),
                task(2, "same", "/profile"),
                task(3, "ambiguous", "/shared"),
                task(4, "same", "/missing"),
                task(5, &AppSessionId::try_from(base)?.to_key(), "/missing"),
            ],
            ..Default::default()
        },
    );
    legacy(&store);
    drop(store);
    let store = StateStore::open_at(&path)?;
    let graph = load(&store.connection);
    let keys: Vec<_> = graph
        .tasks
        .iter()
        .map(|task| task.owner.as_ref().expect("owner").session_id.clone())
        .collect();
    assert_eq!(
        keys,
        [
            AppSessionId::try_from(base)?.to_key(),
            AppSessionId::try_from(profile)?.to_key(),
            "ambiguous".into(),
            "same".into(),
            format!("legacy-session:{}", AppSessionId::try_from(base)?.to_key())
        ]
    );
    assert_eq!(
        graph.sessions[0].session_id,
        AppSessionId::try_from(profile)?.to_key()
    );
    drop(store);
    let store = StateStore::open_at(&path)?;
    assert_eq!(
        load(&store.connection),
        graph,
        "reopening must not reinterpret canonical keys as native IDs"
    );
    Ok(())
}

#[test]
fn migrates_a_bound_draft_whose_legacy_identity_was_its_locator() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let id = session(
        &store,
        "/project",
        "temporary",
        "/bound-draft",
        Backend::Codex,
    );
    store
        .connection
        .execute(
            "UPDATE sessions SET backend_id=NULL,client_key='draft' WHERE id=?1",
            [id],
        )
        .map_err(|error| error.to_string())?;
    graph(
        &store,
        "/project",
        ProjectGraph {
            sessions: vec![SessionLink {
                session_id: "/bound-draft".into(),
                session_path: "/bound-draft".into(),
                plan_number: 1,
                walk_number: 1,
                linked_at: 1,
            }],
            tasks: vec![task(1, "/bound-draft", "/bound-draft")],
            ..Default::default()
        },
    );
    legacy(&store);
    migrate(&mut store.connection)?;
    let graph = load(&store.connection);
    let key = AppSessionId::try_from(id)?.to_key();
    assert_eq!(graph.sessions[0].session_id, key);
    assert_eq!(
        graph.tasks[0].owner.as_ref().expect("owner").session_id,
        key
    );
    Ok(())
}

#[test]
fn merge_updates_legacy_graph_before_removing_its_session_and_rolls_back_atomically()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let keep = session(&store, "/project", "keep", "/keep", Backend::Codex);
    let other = session(&store, "/project", "other", "/other", Backend::Codex);
    graph(
        &store,
        "/project",
        ProjectGraph {
            sessions: vec![SessionLink {
                session_id: "other".into(),
                session_path: "/other".into(),
                plan_number: 1,
                walk_number: 1,
                linked_at: 1,
            }],
            tasks: vec![task(1, "other", "/other")],
            ..Default::default()
        },
    );
    legacy(&store);
    let key = AppSessionId::try_from(keep)?.to_key();
    {
        let tx = store
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        crate::identity::merge_session(&tx, keep, other)?;
        let graph = load(&tx);
        assert_eq!(graph.sessions[0].session_id, key);
        assert_eq!(
            graph.tasks[0].owner.as_ref().expect("owner").session_id,
            key
        );
        assert_eq!(
            tx.query_row(
                "SELECT COUNT(*) FROM sessions WHERE id=?1",
                [other],
                |row| row.get::<_, i64>(0)
            )
            .expect("count removed session"),
            0
        );
    }
    assert_eq!(load(&store.connection).sessions[0].session_id, "other");
    assert!(!migrated(&store.connection)?);
    let tx = store
        .connection
        .transaction()
        .map_err(|error| error.to_string())?;
    crate::identity::merge_session(&tx, keep, other)?;
    tx.commit().map_err(|error| error.to_string())?;
    assert_eq!(load(&store.connection).sessions[0].session_id, key);
    Ok(())
}

#[test]
fn pathless_completion_requires_a_unique_native_session() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let unique = session(&store, "/project", "unique", "/unique", Backend::Codex);
    session(&store, "/project", "same", "/base", Backend::Codex);
    session(&store, "/project", "same", "/profile", Backend::Codex);
    let completed = |number, id: &str| TaskState {
        task: number,
        plan_number: 1,
        owner: None,
        completion: Some(workgraph::TaskCompletion {
            session_id: id.into(),
            completed_at: 1,
            outcome: workgraph::Outcome {
                note: "done".into(),
                evidence: workgraph::Evidence {
                    kind: workgraph::EvidenceKind::Observation,
                    reference: "checked".into(),
                },
            },
        }),
    };
    graph(
        &store,
        "/project",
        ProjectGraph {
            tasks: vec![completed(1, "unique"), completed(2, "same")],
            ..Default::default()
        },
    );
    legacy(&store);
    migrate(&mut store.connection)?;
    let graph = load(&store.connection);
    assert_eq!(
        graph.tasks[0]
            .completion
            .as_ref()
            .expect("completion")
            .session_id,
        AppSessionId::try_from(unique)?.to_key()
    );
    assert_eq!(
        graph.tasks[1]
            .completion
            .as_ref()
            .expect("completion")
            .session_id,
        "same"
    );
    Ok(())
}

#[test]
fn binding_a_draft_moves_graph_ownership_to_the_retained_application_id() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let discovered = session(&store, "/project", "native", "/bound", Backend::Codex);
    let draft = DraftSession::with_id(
        Some(crate::agents::Backend::Codex),
        "draft".into(),
        "/project".into(),
    );
    let retained = store.allocate_app_session_id(&draft)?;
    let old_key = AppSessionId::try_from(discovered)?.to_key();
    let new_key = AppSessionId::try_from(retained)?.to_key();
    workgraph::SqliteAdapter::initialize_connection(&store.connection)
        .map_err(|error| error.to_string())?;
    let edit = |store: &mut StateStore, id: &str, action| {
        store
            .with_connection(|connection| {
                workgraph::WorkGraph::new(workgraph::SqliteAdapter::borrow(connection)).edit(
                    &workgraph::EditRequest {
                        project: "/project".into(),
                        idempotency_key: id.into(),
                        action,
                    },
                )
            })
            .map_err(|error| error.to_string())
    };
    edit(
        &mut store,
        "create",
        workgraph::EditAction::CreateTasks {
            nodes: vec![workgraph::NodeDraft {
                title: "Task".into(),
                acceptance: "Checked".into(),
            }],
            after: None,
            before: None,
        },
    )?;
    edit(
        &mut store,
        "claim",
        workgraph::EditAction::ClaimTask {
            task: 1,
            session_id: old_key.clone(),
            session_path: "/bound".into(),
        },
    )?;
    let tx = store
        .connection
        .transaction()
        .map_err(|error| error.to_string())?;
    crate::identity::bind_locator(&tx, &draft.id, Path::new("/bound"))?;
    tx.commit().map_err(|error| error.to_string())?;
    let graph = load(&store.connection);
    assert_eq!(graph.sessions[0].session_id, new_key);
    assert_eq!(
        graph.tasks[0].owner.as_ref().expect("owner").session_id,
        new_key
    );
    assert!(
        edit(
            &mut store,
            "stale-release",
            workgraph::EditAction::ReleaseTask {
                task: 1,
                session_id: old_key
            }
        )
        .is_err()
    );
    edit(
        &mut store,
        "complete",
        workgraph::EditAction::CompleteTask {
            task: 1,
            session_id: new_key.clone(),
            outcome: workgraph::Outcome {
                note: "checked".into(),
                evidence: workgraph::Evidence {
                    kind: workgraph::EvidenceKind::Observation,
                    reference: "checked".into(),
                },
            },
        },
    )?;
    assert_eq!(
        load(&store.connection).tasks[0]
            .completion
            .as_ref()
            .expect("completion")
            .session_id,
        new_key
    );
    Ok(())
}
