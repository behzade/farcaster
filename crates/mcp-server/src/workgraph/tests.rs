use super::*;
use crate::agents::Backend;
use crate::{
    sessions::{SessionSummary, UsageSummary},
    storage::StateStore,
};
use std::path::Path;

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

fn edit(database: &Path, caller: &CallerContext, action: EditAction) -> Result<EditResult, String> {
    with_test_store(database, |store| super::edit(store, caller, action))
}

fn caller(project: &Path, id: &str) -> CallerContext {
    CallerContext {
        worker_id: format!("worker-{id}"),
        worker_name: id.into(),
        project: project.to_owned(),
        session: project
            .join("sessions")
            .join(id)
            .to_string_lossy()
            .into_owned(),
        session_locator: None,
        harness_profile_id: None,
        app_session_id: None,
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
    for session in ["alice".to_owned(), alice.session.clone()] {
        let mut alias = alice.clone();
        alias.session = session;
        assert_eq!(
            search(&database, &alias, SearchParams::default())?["tasks"][0]["ownedByYou"],
            true
        );
    }
    assert_eq!(
        search(&database, &bob, SearchParams::default())?["tasks"][0]["ownedByYou"],
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
    let alice_key = session_identity(&database, &alice)?.0;
    let selection = with_test_store(&database, |store| {
        store.with_connection(|connection| {
            workgraph::load_plan(connection, temp.path().to_owned(), Some(&alice_key))
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
    assert_eq!(completed["tasks"].as_array().expect("tasks").len(), 1);
    assert_eq!(completed["newlyReady"][0]["status"], "ready");
    assert!(completed["newlyReady"][0]["owner"].is_null());
    let found = search(
        &database,
        &alice,
        SearchParams {
            query: "APPROVED".into(),
            ..Default::default()
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
    let details = search(
        &database,
        &alice,
        SearchParams {
            task: Some(second),
            ..Default::default()
        },
    )?;
    assert_eq!(
        details["tasks"][0]["completion"]["outcome"]["evidence"]["kind"],
        "file"
    );
    assert_eq!(completed["newlyReady"], json!([]));
    Ok(())
}

#[test]
fn duplicate_native_ids_resolve_separate_backend_scoped_keys() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut alice = caller(temp.path(), "alice");
    let mut bob = caller(temp.path(), "bob");
    alice.worker_name = "same-id".into();
    bob.worker_name = "same-id".into();
    bob.backend = Backend::Codex;
    index(&database, &[alice.clone(), bob.clone()])?;
    alice.session = "same-id".into();
    bob.session = "same-id".into();
    assert_ne!(
        session_identity(&database, &alice)?.0,
        session_identity(&database, &bob)?.0
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
    let (profile_key, profile_path) = session_identity(&database, &authenticated)?;
    assert_eq!(
        profile_path,
        crate::sessions::normalize_session_path(&profile).to_string_lossy()
    );
    authenticated.session_locator = None;
    let (base_key, base_path) = session_identity(&database, &authenticated)?;
    assert_ne!(base_key, profile_key);
    assert_eq!(
        base_path,
        crate::sessions::normalize_session_path(&base).to_string_lossy()
    );
    Ok(())
}

#[test]
fn profile_copies_have_separate_owners_and_cannot_release_or_complete_each_other()
-> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let native = "same-native-id";
    let base = temp.path().join("session-locators/claude").join(native);
    let profile = temp
        .path()
        .join("session-locators/profiles/c9eeca98-4e3e-44d5-aabd-9ba354c24e7a/claude")
        .join(native);
    let mut rows = Vec::new();
    let mut callers = Vec::new();
    for path in [&base, &profile] {
        let mut row = caller(temp.path(), native);
        row.backend = Backend::Claude;
        row.session = path.to_string_lossy().into_owned();
        rows.push(row.clone());
        row.session = native.into();
        row.session_locator = Some(path.clone());
        callers.push(row);
    }
    index(&database, &rows)?;
    let base_key = session_identity(&database, &callers[0])?.0;
    let profile_key = session_identity(&database, &callers[1])?.0;
    assert_ne!(base_key, profile_key);
    for title in ["Base task", "Profile task"] {
        patch(
            &database,
            &callers[0],
            PatchParams {
                nodes: vec![PatchNode {
                    title: title.into(),
                    acceptance: "Checked".into(),
                }],
                after: None,
                before: None,
            },
        )?;
    }
    claim(&database, &callers[0], TaskParams { task: 1 })?;
    let result = claim(&database, &callers[1], TaskParams { task: 2 })?;
    assert_eq!(result["tasks"].as_array().expect("tasks").len(), 1);
    assert_eq!(result["tasks"][0]["owner"], profile_key);
    assert_eq!(result["tasks"][0]["ownedByYou"], true);
    let base_task = search(
        &database,
        &callers[1],
        SearchParams {
            task: Some(1),
            ..Default::default()
        },
    )?;
    assert_eq!(base_task["tasks"][0]["owner"], base_key);
    assert_eq!(base_task["tasks"][0]["ownedByYou"], false);
    for (caller, task) in [(&callers[0], 2), (&callers[1], 1)] {
        assert!(release(&database, caller, TaskParams { task }).is_err());
        assert!(
            complete(
                &database,
                caller,
                CompleteParams {
                    task,
                    evidence: "wrong owner".into()
                }
            )
            .is_err()
        );
    }
    let mut wrong_profile = callers[1].clone();
    wrong_profile.harness_profile_id = Some("another-profile".into());
    let mut wrong_app_id = callers[1].clone();
    wrong_app_id.app_session_id = crate::sessions::AppSessionId::from_key(&base_key);
    for caller in [wrong_profile, wrong_app_id] {
        assert!(release(&database, &caller, TaskParams { task: 2 }).is_err());
        assert!(
            complete(
                &database,
                &caller,
                CompleteParams {
                    task: 2,
                    evidence: "conflicting authenticated identity".into()
                }
            )
            .is_err()
        );
    }
    let mut scoped_native = callers[1].clone();
    scoped_native.session_locator = None;
    scoped_native.harness_profile_id = Some("c9eeca98-4e3e-44d5-aabd-9ba354c24e7a".into());
    assert_eq!(session_identity(&database, &scoped_native)?.0, profile_key);
    with_test_store(&database, |store| {
        store.with_connection(|connection| {
            let base = workgraph::load_plan(connection, temp.path().to_owned(), Some(&base_key))?;
            let profile =
                workgraph::load_plan(connection, temp.path().to_owned(), Some(&profile_key))?;
            assert_eq!(base.session_link.expect("base link").session_id, base_key);
            assert_eq!(
                profile.session_link.expect("profile link").session_id,
                profile_key
            );
            Ok(())
        })
    })?;
    complete(
        &database,
        &callers[0],
        CompleteParams {
            task: 1,
            evidence: "base checked".into(),
        },
    )?;
    complete(
        &database,
        &callers[1],
        CompleteParams {
            task: 2,
            evidence: "profile checked".into(),
        },
    )?;
    Ok(())
}

#[test]
fn locatorless_caller_cannot_adopt_another_profiles_only_indexed_session() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let native = "profile-only";
    let profile = temp
        .path()
        .join("session-locators/profiles/c9eeca98-4e3e-44d5-aabd-9ba354c24e7a/claude")
        .join(native);
    let mut row = caller(temp.path(), native);
    row.backend = Backend::Claude;
    row.session = profile.to_string_lossy().into_owned();
    index(&database, std::slice::from_ref(&row))?;
    row.session = native.into();
    assert!(session_identity(&database, &row).is_err());
    row.harness_profile_id = Some("another-profile".into());
    assert!(session_identity(&database, &row).is_err());
    row.harness_profile_id = Some("c9eeca98-4e3e-44d5-aabd-9ba354c24e7a".into());
    assert!(session_identity(&database, &row).is_ok());
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

    let (key, path) = session_identity(&database, &caller)?;
    assert!(crate::sessions::AppSessionId::from_key(&key).is_some());
    assert_eq!(
        path,
        crate::sessions::normalize_session_path(Path::new(&caller.session)).to_string_lossy()
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
        search(&database, &bob, SearchParams::default())?["tasks"],
        json!([])
    );
    assert!(claim(&database, &bob, TaskParams { task: 1 }).is_err());
    Ok(())
}

fn seed_completed_history(
    store: &mut StateStore,
    caller: &CallerContext,
    count: usize,
) -> Result<(), String> {
    use workgraph::{Persistence, TransactionMode, WorkGraphTransaction};
    for start in (0..count).step_by(64) {
        super::edit(
            store,
            caller,
            EditAction::CreateTasks {
                nodes: (start..(start + 64).min(count))
                    .map(|i| NodeDraft {
                        title: format!("Historical task {i}"),
                        acceptance: "Historical acceptance text. ".repeat(50),
                    })
                    .collect(),
                after: None,
                before: None,
            },
        )?;
    }
    let project = project_key(caller)?;
    store.with_connection(|connection| {
        let mut adapter = SqliteAdapter::borrow(connection);
        let mut transaction = adapter
            .begin(TransactionMode::Write)
            .map_err(|e| e.to_string())?;
        let mut stored = transaction
            .project(&project)
            .map_err(|e| e.to_string())?
            .expect("project");
        stored.graph.tasks = stored
            .graph
            .nodes
            .iter()
            .map(|node| workgraph::TaskState {
                plan_number: node.plan_number,
                task: node.number,
                owner: None,
                completion: Some(workgraph::TaskCompletion {
                    session_id: "historical-owner".into(),
                    outcome: Outcome {
                        note: if node.number == 1 {
                            "Distinct historical note".into()
                        } else {
                            "Historical evidence. ".repeat(100)
                        },
                        evidence: Evidence {
                            kind: EvidenceKind::Observation,
                            reference: "Historical evidence. ".repeat(100),
                        },
                    },
                    completed_at: 1,
                }),
            })
            .collect();
        transaction
            .save_project(&project, &stored, 1)
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    })
}

#[test]
fn mutation_replies_stay_small_with_large_history_and_inserted_chains() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let alice = caller(temp.path(), "alice");
    index(&database, std::slice::from_ref(&alice))?;
    with_test_store(&database, |store| {
        seed_completed_history(store, &alice, 300)?;
        let chain = patch_store(
            store,
            &alice,
            PatchParams {
                nodes: vec![
                    PatchNode {
                        title: "Implement".into(),
                        acceptance: "Implement acceptance".into(),
                    },
                    PatchNode {
                        title: "Review".into(),
                        acceptance: "Review acceptance".into(),
                    },
                ],
                after: None,
                before: None,
            },
        )?;
        let first = chain["tasks"][0]["task"].as_u64().expect("first task");
        let second = chain["tasks"][1]["task"].as_u64().expect("second task");
        assert_eq!(first, 301);
        let inserted = patch_store(
            store,
            &alice,
            PatchParams {
                nodes: ["Middle one", "Middle two"]
                    .into_iter()
                    .map(|title| PatchNode {
                        title: title.into(),
                        acceptance: "Middle acceptance".into(),
                    })
                    .collect(),
                after: Some(first),
                before: Some(second),
            },
        )?;
        let inserted_tasks = inserted["tasks"].as_array().expect("inserted tasks");
        assert_eq!(inserted_tasks.len(), 2);
        assert_eq!(inserted_tasks[0]["task"], 303);
        assert_eq!(inserted_tasks[1]["task"], 304);
        assert_eq!(inserted_tasks[0]["blockers"], json!([first]));
        assert_eq!(inserted_tasks[1]["blockers"], json!([303]));
        patch_store(
            store,
            &alice,
            PatchParams {
                nodes: vec![PatchNode {
                    title: "Unrelated ready task".into(),
                    acceptance: "Separate work".into(),
                }],
                after: None,
                before: None,
            },
        )?;
        let claimed = claim_store(store, &alice, TaskParams { task: first })?;
        let released = release_store(store, &alice, TaskParams { task: first })?;
        claim_store(store, &alice, TaskParams { task: first })?;
        let evidence = "Current evidence must be read explicitly. ".repeat(80);
        let completed = complete_store(
            store,
            &alice,
            CompleteParams {
                task: first,
                evidence: evidence.clone(),
            },
        )?;
        for result in [&chain, &inserted, &claimed, &released, &completed] {
            let text = result.to_string();
            assert!(
                text.len() < 800,
                "mutation reply grew to {} bytes",
                text.len()
            );
            assert!(!text.contains("Historical"));
            assert!(!text.contains("acceptance"));
            assert!(!text.contains("evidence"));
        }
        assert_eq!(claimed["tasks"].as_array().expect("tasks").len(), 1);
        assert_eq!(completed["tasks"].as_array().expect("tasks").len(), 1);
        assert_eq!(
            completed["newlyReady"]
                .as_array()
                .expect("newly ready")
                .len(),
            1
        );
        assert_eq!(completed["newlyReady"][0]["task"], 303);
        let details = search_store(
            store,
            &alice,
            SearchParams {
                task: Some(first),
                ..Default::default()
            },
        )?;
        assert_eq!(details["tasks"][0]["acceptance"], "Implement acceptance");
        assert_eq!(
            details["tasks"][0]["completion"]["outcome"]["evidence"]["reference"],
            evidence
        );
        assert!(
            details["tasks"][0]["completion"]["outcome"]
                .get("note")
                .is_none()
        );
        let graph = project_graph(store, &alice)?;
        let persisted = graph
            .task_state(first)
            .expect("task state")
            .completion
            .expect("completion");
        assert_eq!(persisted.outcome.note, evidence);
        assert_eq!(persisted.outcome.evidence.reference, evidence);
        Ok(())
    })
}

#[test]
fn search_pages_active_tasks_and_requires_explicit_history_and_details() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let alice = caller(temp.path(), "alice");
    index(&database, std::slice::from_ref(&alice))?;
    with_test_store(&database, |store| {
        seed_completed_history(store, &alice, 205)?;
        patch_store(
            store,
            &alice,
            PatchParams {
                nodes: (0..30)
                    .map(|i| PatchNode {
                        title: format!("Active task {i}"),
                        acceptance: if i == 15 {
                            "Find NEEDLE in acceptance".into()
                        } else {
                            "Active acceptance".into()
                        },
                    })
                    .collect(),
                after: None,
                before: None,
            },
        )?;
        let mut search = |params| {
            search_store(
                store,
                &alice,
                serde_json::from_value(params).map_err(|e| e.to_string())?,
            )
        };
        let page = search(json!({}))?;
        assert_eq!(page["tasks"].as_array().expect("tasks").len(), 20);
        assert_eq!(page["tasks"][0]["task"], 206);
        assert_eq!(page["nextAfter"], 225);
        assert!(page.to_string().len() < 4000);
        assert!(!page.to_string().contains("acceptance"));
        let next = search(json!({"after": 225}))?;
        assert_eq!(next["tasks"].as_array().expect("tasks").len(), 10);
        assert_eq!(next["tasks"][0]["task"], 226);
        assert!(next["nextAfter"].is_null());
        let history = search(json!({"status": "completed", "limit": 100}))?;
        assert_eq!(history["tasks"].as_array().expect("tasks").len(), 100);
        assert_eq!(history["nextAfter"], 100);
        assert!(!history.to_string().contains("Historical evidence"));
        let all = search(json!({"status": "all", "after": 204, "limit": 2}))?;
        assert_eq!(all["tasks"][0]["status"], "completed");
        assert_eq!(all["tasks"][1]["status"], "ready");
        assert_eq!(all["nextAfter"], 206);
        let ready = search(json!({"status": "ready"}))?;
        assert_eq!(ready["tasks"].as_array().expect("tasks").len(), 1);
        assert!(ready["nextAfter"].is_null());
        let blocked = search(json!({"status": "blocked", "limit": 1}))?;
        assert_eq!(blocked["tasks"][0]["task"], 207);
        assert_eq!(blocked["nextAfter"], 207);
        let found = search(json!({"query": " needle "}))?;
        assert_eq!(found["tasks"].as_array().expect("tasks").len(), 1);
        assert_eq!(found["tasks"][0]["task"], 221);
        let details = search(json!({"task": 1}))?;
        assert_eq!(details["tasks"][0]["status"], "completed");
        assert!(
            details["tasks"][0]["completion"]["outcome"]["evidence"]["reference"]
                .as_str()
                .expect("evidence")
                .contains("Historical evidence")
        );
        assert_eq!(
            details["tasks"][0]["completion"]["outcome"]["note"],
            "Distinct historical note"
        );
        assert!(details["nextAfter"].is_null());
        for invalid in [
            json!({"limit": 0}),
            json!({"limit": 101}),
            json!({"task": 999}),
            json!({"task": 1, "query": "Historical"}),
            json!({"task": 1, "after": 1}),
        ] {
            assert!(search(invalid).is_err());
        }
        Ok(())
    })
}

#[test]
fn completion_reports_only_successors_whose_other_blockers_are_done() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let alice = caller(temp.path(), "alice");
    index(&database, std::slice::from_ref(&alice))?;
    with_test_store(&database, |store| {
        let created = patch_store(
            store,
            &alice,
            PatchParams {
                nodes: ["First", "Second", "Third"]
                    .into_iter()
                    .map(|title| PatchNode {
                        title: title.into(),
                        acceptance: "Done".into(),
                    })
                    .collect(),
                after: None,
                before: None,
            },
        )?;
        super::edit(
            store,
            &alice,
            EditAction::AddEdge {
                plan: created["tasks"][0]["plan"].as_u64().expect("plan"),
                from: 1,
                to: 3,
            },
        )?;
        claim_store(store, &alice, TaskParams { task: 1 })?;
        let completed = complete_store(
            store,
            &alice,
            CompleteParams {
                task: 1,
                evidence: "First done".into(),
            },
        )?;
        assert_eq!(completed["newlyReady"].as_array().expect("ready").len(), 1);
        assert_eq!(completed["newlyReady"][0]["task"], 2);
        let blocked = search_store(
            store,
            &alice,
            SearchParams {
                task: Some(3),
                ..Default::default()
            },
        )?;
        assert_eq!(blocked["tasks"][0]["blockers"], json!([2]));
        claim_store(store, &alice, TaskParams { task: 2 })?;
        let completed = complete_store(
            store,
            &alice,
            CompleteParams {
                task: 2,
                evidence: "Second done".into(),
            },
        )?;
        assert_eq!(completed["newlyReady"].as_array().expect("ready").len(), 1);
        assert_eq!(completed["newlyReady"][0]["task"], 3);
        Ok(())
    })
}
