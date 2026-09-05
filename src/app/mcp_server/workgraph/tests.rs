use super::*;
use crate::{
    app::persistence::StateStore,
    sessions::{SessionSummary, UsageSummary},
};

fn caller(project: &Path, id: &str) -> CallerContext {
    CallerContext {
        worker_id: format!("worker-{id}"),
        worker_name: id.into(),
        project: project.to_owned(),
        session: format!("backend://{id}"),
        backend: "test-backend".into(),
        provider: None,
        model: None,
        effort: None,
        parent_worker_id: None,
    }
}

fn index(database: &Path, callers: &[CallerContext]) -> Result<(), String> {
    let sessions = callers
        .iter()
        .map(|caller| {
            SessionSummary::from_cached_for_harness(
                caller.worker_name.clone(),
                caller.backend.clone(),
                caller.session.clone().into(),
                caller.project.clone(),
                caller.worker_name.clone(),
                String::new(),
                String::new(),
                None,
                UNIX_EPOCH,
                0,
                UsageSummary::default(),
                false,
                false,
                String::new(),
            )
        })
        .collect::<Vec<_>>();
    StateStore::open_at(database)?.replace_sessions(&sessions)
}

#[test]
fn task_lifecycle_uses_authenticated_identity_and_shared_database() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let alice = caller(temp.path(), "alice");
    let bob = caller(temp.path(), "bob");
    index(&database, &[alice.clone(), bob.clone()])?;
    let created = patch(
        &database,
        &alice,
        PatchParams {
            nodes: vec![
                PatchNode {
                    title: "Implement".into(),
                    acceptance: "Tests pass".into(),
                },
                PatchNode {
                    title: "Review".into(),
                    acceptance: "Review approved".into(),
                },
            ],
            after: None,
            before: None,
        },
    )?;
    let first = created["tasks"][0]["task"].as_u64().unwrap();
    let second = created["tasks"][1]["task"].as_u64().unwrap();
    assert_eq!(created["tasks"][0]["status"], "ready");
    assert!(created["tasks"][0]["owner"].is_null());
    assert_eq!(created["tasks"][1]["blockers"], json!([first]));
    assert!(claim(&database, &bob, TaskParams { task: second }).is_err());
    let claimed = claim(&database, &alice, TaskParams { task: first })?;
    assert_eq!(claimed["tasks"][0]["ownedByYou"], true);
    assert_eq!(
        search(
            &database,
            &bob,
            SearchParams {
                query: String::new()
            }
        )?["tasks"][0]["ownedByYou"],
        false
    );
    claim(&database, &alice, TaskParams { task: first })?;
    assert!(claim(&database, &bob, TaskParams { task: first }).is_err());
    assert!(release(&database, &bob, TaskParams { task: first }).is_err());
    assert!(
        complete(
            &database,
            &bob,
            CompleteParams {
                task: first,
                evidence: "wrong owner".into()
            }
        )
        .is_err()
    );
    // The same catalog identity is used by the right sidebar.
    let selection = workgraph::load_plan(database.clone(), temp.path().to_owned(), Some("alice"))?;
    assert_eq!(
        selection.snapshot.unwrap().walk.unwrap().current_node,
        Some(first)
    );
    release(&database, &alice, TaskParams { task: first })?;
    claim(&database, &bob, TaskParams { task: first })?;
    let completed = complete(
        &database,
        &bob,
        CompleteParams {
            task: first,
            evidence: "Tests passed".into(),
        },
    )?;
    assert_eq!(completed["tasks"][0]["status"], "completed");
    assert_eq!(completed["newlyReady"][0]["task"], second);
    assert_eq!(completed["tasks"][1]["status"], "ready");
    assert!(completed["tasks"][1]["owner"].is_null());
    let found = search(
        &database,
        &alice,
        SearchParams {
            query: "APPROVED".into(),
        },
    )?;
    assert_eq!(found["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(found["tasks"][0]["task"], second);
    assert!(!found.to_string().contains("sessionPath"));
    // Existing file-evidence tasks remain completable through the compact API.
    edit(
        &database,
        &alice,
        EditAction::SetNode {
            plan: created["tasks"][1]["plan"].as_u64().unwrap(),
            number: second,
            title: None,
            files: None,
            completion: Some(workgraph::CompletionRequirement::File),
            expected_version: None,
        },
    )?;
    claim(&database, &alice, TaskParams { task: second })?;
    let completed = complete(
        &database,
        &alice,
        CompleteParams {
            task: second,
            evidence: "review.md".into(),
        },
    )?;
    assert_eq!(
        completed["tasks"][1]["completion"]["outcome"]["evidence"]["kind"],
        "file"
    );
    assert_eq!(completed["newlyReady"], json!([]));
    Ok(())
}

#[test]
fn duplicate_backend_ids_cannot_share_task_ownership() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut alice = caller(temp.path(), "alice");
    let mut bob = caller(temp.path(), "bob");
    alice.worker_name = "same-id".into();
    bob.worker_name = "same-id".into();
    bob.backend = "another-backend".into();
    index(&database, &[alice.clone(), bob.clone()])?;
    assert!(
        session_identity(&database, &alice)
            .unwrap_err()
            .contains("ambiguous")
    );
    assert!(
        session_identity(&database, &bob)
            .unwrap_err()
            .contains("ambiguous")
    );
    Ok(())
}

#[test]
fn caller_cannot_claim_a_different_projects_task() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let other = temp.path().join("other");
    std::fs::create_dir(&other).map_err(|e| e.to_string())?;
    let alice = caller(temp.path(), "alice");
    let bob = caller(&other, "bob");
    index(&database, &[alice.clone(), bob.clone()])?;
    patch(
        &database,
        &alice,
        PatchParams {
            nodes: vec![PatchNode {
                title: "Private project task".into(),
                acceptance: "Done".into(),
            }],
            after: None,
            before: None,
        },
    )?;
    assert_eq!(
        search(
            &database,
            &bob,
            SearchParams {
                query: String::new()
            }
        )?["tasks"],
        json!([])
    );
    assert!(claim(&database, &bob, TaskParams { task: 1 }).is_err());
    Ok(())
}
