use super::*;
use crate::agents::Backend;
use crate::{
    sessions::{SessionSummary, UsageSummary},
    storage::StateStore,
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

fn with_test_store<T>(
    database: &Path,
    operation: impl FnOnce(&mut StateStore) -> Result<T, String>,
) -> Result<T, String> {
    let mut store = StateStore::open_at(database)?;
    store.with_connection(|connection| {
        SqliteAdapter::initialize_connection(connection).map_err(|error| error.to_string())
    })?;
    operation(&mut store)
}

fn session_identity(database: &Path, caller: &CallerContext) -> Result<(String, String), String> {
    with_test_store(database, |store| session_identity_store(store, caller))
}

fn search(database: &Path, caller: &CallerContext, params: SearchParams) -> Result<Value, String> {
    with_test_store(database, |store| search_store(store, caller, params))
}

fn patch(database: &Path, caller: &CallerContext, params: PatchParams) -> Result<Value, String> {
    with_test_store(database, |store| patch_store(store, caller, params))
}

fn claim(database: &Path, caller: &CallerContext, params: TaskParams) -> Result<Value, String> {
    with_test_store(database, |store| claim_store(store, caller, params))
}

fn release(database: &Path, caller: &CallerContext, params: TaskParams) -> Result<Value, String> {
    with_test_store(database, |store| release_store(store, caller, params))
}

fn complete(
    database: &Path,
    caller: &CallerContext,
    params: CompleteParams,
) -> Result<Value, String> {
    with_test_store(database, |store| complete_store(store, caller, params))
}

fn edit(database: &Path, caller: &CallerContext, action: EditAction) -> Result<Value, String> {
    with_test_store(database, |store| super::edit(store, caller, action))
}

fn caller(project: &Path, id: &str) -> CallerContext {
    CallerContext {
        worker_id: format!("worker-{id}"),
        worker_name: id.into(),
        project: project.to_owned(),
        session: format!("backend://{id}"),
        session_locator: None,
        backend: Backend::Pi,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    }
}

fn index(database: &Path, callers: &[CallerContext]) -> Result<(), String> {
    let sessions = callers
        .iter()
        .map(|caller| {
            SessionSummary::from_cached_for_harness(
                caller.worker_name.clone(),
                caller.backend,
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
    StateStore::open_at(database)?.index_sessions(&sessions, true)
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
    let first = created["tasks"][0]["task"]
        .as_u64()
        .expect("test operation should succeed");
    let second = created["tasks"][1]["task"]
        .as_u64()
        .expect("test operation should succeed");
    assert_eq!(created["tasks"][0]["status"], "ready");
    assert!(created["tasks"][0]["owner"].is_null());
    assert_eq!(created["tasks"][1]["blockers"], json!([first]));
    assert!(claim(&database, &bob, TaskParams { task: second }).is_err());
    let claimed = claim(&database, &alice, TaskParams { task: first })?;
    assert_eq!(claimed["tasks"][0]["ownedByYou"], true);
    for session in ["alice", "backend:/alice"] {
        let mut alias = alice.clone();
        alias.session = session.into();
        assert_eq!(
            search(
                &database,
                &alias,
                SearchParams {
                    query: String::new()
                }
            )?["tasks"][0]["ownedByYou"],
            true
        );
    }
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
    let selection = with_test_store(&database, |store| {
        store.with_connection(|connection| {
            workgraph::load_plan(connection, temp.path().to_owned(), Some("alice"))
        })
    })?;
    assert_eq!(
        selection
            .snapshot
            .expect("test operation should succeed")
            .walk
            .expect("test operation should succeed")
            .current_node,
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
    assert_eq!(
        found["tasks"]
            .as_array()
            .expect("test operation should succeed")
            .len(),
        1
    );
    assert_eq!(found["tasks"][0]["task"], second);
    assert!(!found.to_string().contains("sessionPath"));
    edit(
        &database,
        &alice,
        EditAction::SetNode {
            plan: created["tasks"][1]["plan"]
                .as_u64()
                .expect("test operation should succeed"),
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
    bob.backend = Backend::Codex;
    index(&database, &[alice.clone(), bob.clone()])?;
    assert!(
        session_identity(&database, &alice)
            .expect_err("invalid test input must fail")
            .contains("ambiguous")
    );
    assert!(
        session_identity(&database, &bob)
            .expect_err("invalid test input must fail")
            .contains("ambiguous")
    );
    Ok(())
}

#[test]
fn profiled_caller_locator_resolves_duplicate_native_ids() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let id = "d093cd84-7700-4ec2-be8f-8a1b079d6684";
    let base = temp.path().join("session-locators/claude").join(id);
    let profile = temp
        .path()
        .join("session-locators/profiles/c9eeca98-4e3e-44d5-aabd-9ba354c24e7a/claude")
        .join(id);
    let mut base_row = caller(temp.path(), id);
    base_row.backend = Backend::Claude;
    base_row.worker_name = id.into();
    base_row.session = base.to_string_lossy().into_owned();
    let mut profile_row = base_row.clone();
    profile_row.session = profile.to_string_lossy().into_owned();
    index(&database, &[base_row, profile_row])?;

    let mut authenticated = caller(temp.path(), id);
    authenticated.backend = Backend::Claude;
    authenticated.session = id.into();
    authenticated.session_locator = Some(profile.clone());
    assert_eq!(
        session_identity(&database, &authenticated)?.1,
        profile.to_string_lossy()
    );
    authenticated.session_locator = None;
    assert!(session_identity(&database, &authenticated).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn session_identity_accepts_an_authenticated_project_alias() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let project = temp.path().join("project");
    let alias = temp.path().join("project-alias");
    std::fs::create_dir(&project).map_err(|error| error.to_string())?;
    symlink(&project, &alias).map_err(|error| error.to_string())?;
    let caller = caller(&alias, "alice");
    index(&database, std::slice::from_ref(&caller))?;

    assert_eq!(
        session_identity(&database, &caller)?,
        (
            "alice".into(),
            crate::sessions::normalize_session_path(Path::new(&caller.session))
                .to_string_lossy()
                .into_owned(),
        )
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
