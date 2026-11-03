use super::*;
use serde_json::json;

#[test]
fn caller_binding_publishes_child_and_family_together() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = crate::sessions::normalize_session_path(temp.path());
    let database = temp.path().join("state.sqlite3");
    let store = SharedStateStore::new(StateStore::open_at(&database)?);
    let reader = SharedStateStore::new(StateStore::open_at(&database)?);
    let registry = crate::agents::CallerRegistry::default();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let registrations = store.clone();
    let catalog = reader.clone();
    let during_registration = captured.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |caller, family| {
            let id = registrations.with(|store| store.register_caller_session(caller, family))?;
            if caller.parent_worker_id.is_some() {
                // Read as soon as registration returns, before any later family
                // callback could hide an incorrectly published root.
                let rows = catalog.with(|store| store.cached_sessions(""))?;
                during_registration.lock().expect("capture").push(rows);
            }
            Ok(id)
        })),
        None,
    );
    let family_writes = Arc::new(Mutex::new(0));
    let writes = family_writes.clone();
    let families = store.clone();
    registry.set_family_sink(Some(Arc::new(move |link| {
        *writes.lock().expect("family writes") += 1;
        families.with(|store| store.save_worker_family(link))
    })));
    let profile = crate::agents::CallerProfile {
        backend: Backend::Codex,
        provider: None,
        model: None,
        effort: None,
    };
    let parent = registry.issue_with_access(
        &project,
        profile.clone(),
        None,
        crate::agents::HarnessAccessMode::Auto,
    );
    parent.bind("parent");
    let parent_context = registry.resolve(parent.token())?;
    let parent_row = reader.with(|store| store.resolve_caller_session(&parent_context))?;
    let before =
        crate::sessions::SessionCatalog::from(reader.with(|store| store.cached_sessions(""))?);
    assert_eq!(before.roots().count(), 1);

    let child = registry.issue_as_with_access(
        &project,
        profile,
        None,
        "worker-child".into(),
        "child".into(),
        Some(parent_context.worker_id),
        crate::agents::HarnessAccessMode::Auto,
    )?;
    child.bind("child");

    let snapshots = captured.lock().expect("capture");
    assert_eq!(snapshots.len(), 1);
    let during = crate::sessions::SessionCatalog::from(snapshots[0].clone());
    let roots = during.roots().collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].app_session_id, parent_row.app_session_id);
    let child_row = during.iter().find(|row| row.id == "child").expect("child");
    assert!(child_row.title.is_empty());
    assert_eq!(
        child_row.parent_app_session_id,
        Some(parent_row.app_session_id)
    );
    drop(snapshots);

    let after =
        crate::sessions::SessionCatalog::from(reader.with(|store| store.cached_sessions(""))?);
    let roots = after.roots().collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].app_session_id, parent_row.app_session_id);
    let linked = after.iter().find(|row| row.id == "child").expect("child");
    assert_eq!(linked.app_session_id, child_row.app_session_id);
    assert_eq!(
        linked.parent_app_session_id,
        Some(parent_row.app_session_id)
    );
    assert!(linked.title.is_empty());

    // Rebinding a row whose family is already stored does not expose it again.
    child.bind("child");
    let snapshots = captured.lock().expect("capture");
    assert_eq!(snapshots.len(), 2);
    let rebound = crate::sessions::SessionCatalog::from(snapshots[1].clone());
    assert_eq!(rebound.roots().count(), 1);
    drop(snapshots);
    assert_eq!(*family_writes.lock().expect("family writes"), 0);

    let assignment = crate::agents::WorkerAssignment {
        profile: "review".into(),
        harness_profile_id: None,
        execution: crate::agents::WorkerExecution {
            harness: Backend::Codex,
            provider: "openai".into(),
            model: "initial".into(),
            effort: None,
            service_tier: None,
        },
    };
    registry.set_assignment("worker-child", assignment.clone())?;
    child.select_model("openai", "selected");
    child.select_effort("high");
    let routes = reader.with(|store| store.load_worker_routes())?;
    assert_eq!(routes.len(), 1);
    assert_eq!(
        routes[0].routing.as_ref().expect("routing").assignment,
        assignment
    );
    let execution = routes[0].execution.as_ref().expect("execution");
    assert_eq!(execution.model, "selected");
    assert_eq!(execution.effort.as_deref(), Some("high"));

    // Switching back to a saved child must move the live route and selection
    // to that row, rather than leaving restart pointed at the previous session.
    child.bind("other-child");
    child.select_model("openai", "updated");
    child.select_effort("low");
    child.bind("child");
    let routes = reader
        .with(|store| store.load_worker_routes())?
        .into_iter()
        .filter(|link| link.routing.is_some())
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].child_key, Some(child_row.key()));
    let execution = routes[0].execution.as_ref().expect("execution");
    assert_eq!(execution.model, "updated");
    assert_eq!(execution.effort.as_deref(), Some("low"));
    Ok(())
}

#[test]
fn caller_registration_rolls_back_when_family_cannot_be_saved() -> Result<(), String> {
    for register_turn in [false, true] {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
        let parent = caller(temp.path(), "parent");
        let parent_id = store.register_caller_session(&parent, None)?;
        let mut child = caller(temp.path(), "child");
        child.backend = Backend::Codex;
        child.harness_profile_id = Some("11111111-1111-4111-8111-111111111111".into());
        child.parent_worker_id = Some(parent.worker_id.clone());
        let turn = crate::agents::ExecutionBinding {
            session_record: 999,
            turn_id: "turn".into(),
            prompt_id: Some("prompt".into()),
        };
        let register = |family: Option<&crate::agents::WorkerFamilyLink>| {
            if register_turn {
                store.register_execution_for_caller(&child, &turn, family)
            } else {
                store.register_caller_session(&child, family)
            }
        };
        assert!(
            register(None).is_err(),
            "unbound parent must defer registration"
        );
        let mut family = crate::agents::WorkerFamilyLink {
            project: temp.path().into(),
            child_backend: child.backend,
            child_session: child.session.clone(),
            parent_backend: parent.backend,
            parent_session: parent.session.clone(),
            child_key: None,
            parent_key: crate::sessions::AppSessionId::new(parent_id + 1000)
                .map(crate::sessions::SessionKey::App),
            execution: None,
            routing: None,
        };
        assert!(
            register(Some(&family)).is_err(),
            "stale parent key must fail"
        );
        assert_eq!(store.cached_sessions("")?.len(), 1);

        family.parent_key =
            crate::sessions::AppSessionId::new(parent_id).map(crate::sessions::SessionKey::App);
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_family BEFORE INSERT ON worker_families
             BEGIN SELECT RAISE(ABORT, 'family write failed'); END;",
            )
            .map_err(|error| error.to_string())?;
        let error = register(Some(&family)).expect_err("family failure must roll back child");
        assert!(error.contains("family write failed"), "{error}");
        assert_eq!(store.cached_sessions("")?.len(), 1);
        let turns: i64 = store
            .connection
            .query_row("SELECT count(*) FROM session_turns", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        assert_eq!(turns, 0);

        store
            .connection
            .execute_batch("DROP TRIGGER fail_family")
            .map_err(|error| error.to_string())?;
        if register_turn {
            store
                .connection
                .execute_batch(
                    "CREATE TRIGGER fail_turn BEFORE INSERT ON session_turns
                 BEGIN SELECT RAISE(ABORT, 'turn write failed'); END;",
                )
                .map_err(|error| error.to_string())?;
            let error = register(Some(&family)).expect_err("turn failure must roll back family");
            assert!(error.contains("turn write failed"), "{error}");
            assert_eq!(store.cached_sessions("")?.len(), 1);
            assert!(store.load_worker_families()?.is_empty());
            store
                .connection
                .execute_batch("DROP TRIGGER fail_turn")
                .map_err(|error| error.to_string())?;
        }
        let child_id = register(Some(&family))?;
        let catalog = crate::sessions::SessionCatalog::from(store.cached_sessions("")?);
        assert_eq!(catalog.roots().count(), 1);
        let row = catalog
            .iter()
            .find(|row| row.app_session_id == child_id)
            .expect("child");
        assert_eq!(row.parent_app_session_id, Some(parent_id));
        assert_eq!(row.profile_id, child.harness_profile_id);
        let links = store.load_worker_families()?;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].child_key.as_ref(), Some(&row.key()));
        let turns: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM session_turns WHERE session_id=?1",
                [child_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        assert_eq!(turns, i64::from(register_turn));
    }
    Ok(())
}

#[test]
fn child_binding_waits_for_parent_registration() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = SharedStateStore::new(StateStore::open_at(&temp.path().join("state.sqlite3"))?);
    let registry = crate::agents::CallerRegistry::default();
    let registrations = store.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |caller, family| {
            registrations.with(|store| store.register_caller_session(caller, family))
        })),
        None,
    );
    let families = store.clone();
    registry.set_family_sink(Some(Arc::new(move |link| {
        families.with(|store| store.save_worker_family(link))
    })));
    let profile = crate::agents::CallerProfile {
        backend: Backend::Codex,
        provider: None,
        model: None,
        effort: None,
    };
    let parent = registry.issue_with_access(
        temp.path(),
        profile.clone(),
        None,
        crate::agents::HarnessAccessMode::Auto,
    );
    let child = registry.issue_as_with_access(
        temp.path(),
        profile,
        None,
        "worker-child".into(),
        "child".into(),
        Some(parent.worker_identity().expect("parent").0),
        crate::agents::HarnessAccessMode::Auto,
    )?;
    child.bind("child");
    assert!(store.with(|store| store.cached_sessions(""))?.is_empty());
    parent.bind("parent");
    let catalog =
        crate::sessions::SessionCatalog::from(store.with(|store| store.cached_sessions(""))?);
    assert_eq!(catalog.len(), 2);
    let root = catalog.roots().next().expect("parent");
    assert_eq!(catalog.roots().count(), 1);
    assert_eq!(root.id, "parent");
    let child = catalog.iter().find(|row| row.id == "child").expect("child");
    assert_eq!(child.parent_app_session_id, Some(root.app_session_id));
    Ok(())
}

#[test]
fn linked_child_registers_turns_after_parent_merge_without_rewriting_family() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let project = crate::sessions::normalize_session_path(temp.path());
    let store = SharedStateStore::new(StateStore::open_at(&temp.path().join("state.sqlite3"))?);
    let registry = crate::agents::CallerRegistry::default();
    let registrations = store.clone();
    let turns = store.clone();
    registry.set_execution_sinks(
        Some(Arc::new(move |caller, family| {
            registrations.with(|store| store.register_caller_session(caller, family))
        })),
        Some(Arc::new(move |caller, turn, family| {
            turns.with(|store| store.register_execution_for_caller(caller, turn, family))
        })),
    );
    let mut draft = DraftSession::fresh(Some(Backend::Pi), project.clone());
    let canonical_id = store.with(|store| store.allocate_app_session_id(&draft))?;
    let profile = crate::agents::CallerProfile {
        backend: Backend::Pi,
        provider: None,
        model: None,
        effort: None,
    };
    let parent = registry.issue_with_access(
        &project,
        profile.clone(),
        None,
        crate::agents::HarnessAccessMode::Auto,
    );
    let parent_path = project.join("parent.jsonl");
    parent.bind(parent_path.to_string_lossy().into_owned());
    let parent_context = registry.resolve(parent.token())?;
    assert_ne!(
        parent_context.app_session_id.expect("parent ID").get(),
        canonical_id
    );
    let child = registry.issue_as_with_access(
        &project,
        profile,
        None,
        "worker-child".into(),
        "child".into(),
        Some(parent_context.worker_id),
        crate::agents::HarnessAccessMode::Auto,
    )?;
    child.bind(project.join("child.jsonl").to_string_lossy().into_owned());
    child.begin_execution(Some("before-merge"));
    registry.resolve_execution(child.token())?;
    draft.app_session_id = canonical_id;
    draft.session_path = Some(parent_path);
    store.with(|store| store.allocate_app_session_id(&draft))?;
    let saved = store.with(|store| store.load_worker_families())?;
    assert_eq!(
        saved[0].parent_key,
        crate::sessions::AppSessionId::new(canonical_id).map(crate::sessions::SessionKey::App)
    );

    // The child's registry snapshot still has the old parent ID. Neither a
    // parent refresh nor a live parent caller is required to register a turn.
    child.begin_execution(Some("after-merge"));
    let (_, turn) = registry.resolve_execution(child.token())?;
    assert_eq!(turn.prompt_id.as_deref(), Some("after-merge"));
    assert_eq!(store.with(|store| store.load_worker_families())?, saved);
    drop(parent);
    child.begin_execution(Some("after-parent-exit"));
    let (_, turn) = registry.resolve_execution(child.token())?;
    assert_eq!(turn.prompt_id.as_deref(), Some("after-parent-exit"));
    assert_eq!(store.with(|store| store.load_worker_families())?, saved);
    Ok(())
}

fn caller(project: &Path, session: &str) -> crate::agents::CallerContext {
    crate::agents::CallerContext {
        worker_id: "worker".into(),
        worker_name: "Worker".into(),
        project: project.into(),
        session: session.into(),
        session_locator: None,
        harness_profile_id: None,
        app_session_id: None,
        backend: Backend::Cursor,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    }
}
fn artifact(project: &Path, id: &str) -> serde_json::Value {
    json!({"farcaster_review":{"version":1,"id":id,"project":project,"review":{"title":"Review","items":[{"path":"README.md","note":"Inspect"}]}}})
}
fn execution(
    store: &StateStore,
    caller: &crate::agents::CallerContext,
    turn: &str,
) -> crate::agents::ExecutionBinding {
    let mut execution = crate::agents::ExecutionBinding {
        session_record: store
            .register_caller_session(caller, None)
            .expect("register session"),
        turn_id: turn.into(),
        prompt_id: Some(turn.into()),
    };
    execution.session_record = store
        .register_execution_for_caller(caller, &execution, None)
        .expect("register turn");
    execution
}

const CLAUDE_ID: &str = "d093cd84-7700-4ec2-be8f-8a1b079d6684";

fn profiled_claude_path(project: &Path) -> PathBuf {
    project.join(format!(
        "session-locators/profiles/c9eeca98-4e3e-44d5-aabd-9ba354c24e7a/claude/{CLAUDE_ID}"
    ))
}

fn live_claude_metadata(project: &Path) -> crate::agents::SessionMetadata {
    crate::agents::SessionMetadata {
        profile_id: None,
        harness: Backend::Claude,
        id: CLAUDE_ID.into(),
        path: profiled_claude_path(project),
        project: project.into(),
        title: Some("Live Claude session".into()),
        first_user_message: Some("prompt".into()),
        parent_session: None,
        forked_from: None,
        message_count: Some(2),
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: None,
        usage: None,
        is_running: false,
    }
}

#[test]
fn profiled_caller_uses_the_live_locator_and_archive_row() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let locator = profiled_claude_path(temp.path());
    let mut caller = caller(temp.path(), CLAUDE_ID);
    caller.backend = Backend::Claude;
    caller.session_locator = Some(locator.clone());

    let provisioned = store.register_caller_session(&caller, None)?;
    let session = store.update_session_metadata(&live_claude_metadata(temp.path()))?;
    assert_eq!(session.app_session_id, provisioned);
    let turn = execution(&store, &caller, "profiled-turn");
    assert_eq!(turn.session_record, provisioned);
    store.save_review(&caller, &turn, &artifact(temp.path(), "profiled-review"))?;
    store.set_session_archived(&locator, true)?;

    let sessions = store.cached_sessions("")?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].path,
        crate::sessions::normalize_session_path(&locator)
    );
    assert!(sessions[0].archived);
    assert_eq!(sessions[0].message_count, 2);
    caller.session_locator = Some(temp.path().join("session-locators/claude/different"));
    assert!(store.register_caller_session(&caller, None).is_err());
    assert_eq!(store.cached_sessions("")?.len(), 1);
    Ok(())
}

#[test]
fn old_profiled_caller_placeholder_moves_turns_without_archiving_live_session() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let database = temp.path().join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let locator = profiled_claude_path(temp.path());
    let real = store.update_session_metadata(&live_claude_metadata(temp.path()))?;
    let ghost_locator = super::super::identity::family_locator_root(
        &temp.path().join("session-locators"),
        temp.path(),
    )
    .join("claude")
    .join(CLAUDE_ID);
    let ghost_locator = crate::sessions::normalize_session_path(&ghost_locator);
    store.connection.execute(
        "INSERT INTO sessions(project_id,harness,backend_id,locator,modified_ms,created_ms,archived_at)
         SELECT project_id,harness,backend_id,?2,0,0,1 FROM sessions WHERE id=?1",
        params![real.app_session_id, ghost_locator.to_string_lossy().as_ref()],
    ).map_err(|error| error.to_string())?;
    let ghost_id = store.connection.last_insert_rowid();
    store
        .connection
        .execute(
            "INSERT INTO session_turns(id,session_id,started_ms) VALUES('legacy-turn',?1,0)",
            [ghost_id],
        )
        .map_err(|error| error.to_string())?;
    store
        .connection
        .execute(
            "DELETE FROM meta WHERE key='profiled_caller_identity_repaired_v1'",
            [],
        )
        .map_err(|error| error.to_string())?;
    drop(store);

    let store = StateStore::open_at(&database)?;
    let sessions = store.cached_sessions("")?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].app_session_id, real.app_session_id);
    assert_eq!(
        sessions[0].path,
        crate::sessions::normalize_session_path(&locator)
    );
    assert_eq!(sessions[0].message_count, 2);
    assert!(!sessions[0].archived);
    let turn_owner: i64 = store
        .connection
        .query_row(
            "SELECT session_id FROM session_turns WHERE id='legacy-turn'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(turn_owner, real.app_session_id);
    Ok(())
}

#[test]
fn review_submission_is_read_only_for_session_identity_and_uses_captured_turn() -> Result<(), String>
{
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let first = execution(&store, &caller, "first");
    let before: (i64, String) = store
        .connection
        .query_row("SELECT id,locator FROM sessions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("session");
    store.save_review(&caller, &first, &artifact(temp.path(), "review"))?;
    let after: (i64, String) = store
        .connection
        .query_row("SELECT id,locator FROM sessions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("session");
    assert_eq!(before, after);
    let saved: String = store
        .connection
        .query_row(
            "SELECT turn_id FROM session_reviews WHERE id='review'",
            [],
            |r| r.get(0),
        )
        .expect("review row");
    assert_eq!(saved, "first");
    Ok(())
}

#[test]
fn reviews_survive_identity_merges_and_refuse_stranger_callers() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let execution = execution(&store, &caller, "turn");
    let other = self::caller(temp.path(), "other");
    let keep = store.register_caller_session(&other, None)?;
    let tx = store
        .connection
        .unchecked_transaction()
        .expect("transaction");
    super::super::identity::merge_session(&tx, keep, execution.session_record)?;
    tx.commit().expect("merge");
    store.save_review(&caller, &execution, &artifact(temp.path(), "review"))?;
    drop(store);
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let saved: i64 = store
        .connection
        .query_row("SELECT count(*) FROM session_reviews", [], |r| r.get(0))
        .expect("count");
    assert_eq!(saved, 1, "the merge kept the saved review");
    assert!(
        store
            .save_review(
                &self::caller(temp.path(), "stranger"),
                &execution,
                &artifact(temp.path(), "wrong")
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn turn_registration_resolves_a_session_merged_after_caller_binding() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let stale = store.register_caller_session(&caller, None)?;
    let keep = store.register_caller_session(&self::caller(temp.path(), "other"), None)?;
    let tx = store
        .connection
        .unchecked_transaction()
        .expect("transaction");
    super::super::identity::merge_session(&tx, keep, stale)?;
    tx.commit().expect("merge");

    let turn = crate::agents::ExecutionBinding {
        session_record: stale,
        turn_id: "after-merge".into(),
        prompt_id: Some("prompt".into()),
    };
    assert_eq!(
        store.register_execution_for_caller(&caller, &turn, None)?,
        keep
    );
    let owner: i64 = store
        .connection
        .query_row(
            "SELECT session_id FROM session_turns WHERE id=?1",
            [&turn.turn_id],
            |row| row.get(0),
        )
        .expect("registered turn");
    assert_eq!(owner, keep);
    store.save_review(&caller, &turn, &artifact(temp.path(), "after-merge-review"))?;
    Ok(())
}

#[test]
fn failed_review_write_does_not_provision_sessions_or_claim_success() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let caller = caller(temp.path(), "native");
    let execution = execution(&store, &caller, "turn");
    store.connection.execute_batch("CREATE TRIGGER reject_review BEFORE INSERT ON session_reviews BEGIN SELECT RAISE(FAIL,'disk failure'); END;").expect("trigger");
    assert!(
        store
            .save_review(&caller, &execution, &artifact(temp.path(), "review"))
            .is_err()
    );
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM session_reviews", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 0);
    Ok(())
}

#[test]
fn migration_preserves_legacy_artifacts_without_guessing_a_turn() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("project");
    let database = temp.path().join("state.sqlite3");
    let store = StateStore::open_at(&database)?;
    let caller = caller(temp.path(), "native");
    let record = store.register_caller_session(&caller, None)?;
    let body = json!({"type":"review_submitted","submission":{"artifact":artifact(temp.path(),"legacy"),"prompt_id":"ambiguous"}});
    store
        .connection
        .execute(
            "INSERT INTO session_events VALUES(?1,1,1,1,?2)",
            params![record, body.to_string()],
        )
        .expect("legacy record");
    store.connection.execute_batch("DROP TABLE session_reviews; DROP TABLE session_turns; UPDATE meta SET value='16' WHERE key='schema_version';").expect("old schema");
    drop(store);
    let store = StateStore::open_at(&database)?;
    let saved: String = store
        .connection
        .query_row(
            "SELECT json_extract(artifact,'$.farcaster_review.id') FROM session_reviews",
            [],
            |r| r.get(0),
        )
        .expect("legacy review");
    assert_eq!(saved, "legacy");
    Ok(())
}

#[test]
fn identical_backend_ids_in_different_projects_keep_distinct_review_owners() -> Result<(), String> {
    let temp = tempfile::tempdir().expect("storage");
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir_all(&first).expect("first project");
    std::fs::create_dir_all(&second).expect("second project");
    let a = caller(&first, "native");
    let b = caller(&second, "native");
    let turn_a = execution(&store, &a, "first-turn");
    let turn_b = execution(&store, &b, "second-turn");
    assert_ne!(turn_a.session_record, turn_b.session_record);
    assert!(
        store
            .save_review(&b, &turn_a, &artifact(&second, "wrong-project"))
            .is_err()
    );
    store.save_review(&a, &turn_a, &artifact(&first, "first-review"))?;
    store.save_review(&b, &turn_b, &artifact(&second, "second-review"))?;
    Ok(())
}

#[test]
fn named_pi_caller_registration_preserves_explicit_profile_without_metadata() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let mut context = caller(
        temp.path(),
        &temp.path().join("native-session.jsonl").to_string_lossy(),
    );
    context.backend = Backend::Pi;
    context.harness_profile_id = Some("11111111-1111-4111-8111-111111111111".into());
    let id = store.register_caller_session(&context, None)?;
    let profile: Option<String> = store
        .connection
        .query_row("SELECT profile_id FROM sessions WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    assert_eq!(profile, context.harness_profile_id);
    context.app_session_id = crate::sessions::AppSessionId::new(id + 1000);
    assert_eq!(
        store.register_caller_session(&context, None)?,
        id,
        "stale app IDs are not authoritative"
    );
    context.harness_profile_id = Some("22222222-2222-4222-8222-222222222222".into());
    assert!(
        store.register_caller_session(&context, None).is_err(),
        "conflicting profile must not retarget the row"
    );
    Ok(())
}

#[test]
fn named_native_only_caller_reuses_indexed_profile_session() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let profile = "11111111-1111-4111-8111-111111111111";
    let mut context = caller(temp.path(), "existing-native");
    context.backend = Backend::Codex;
    context.harness_profile_id = Some(profile.into());
    let metadata = crate::agents::SessionMetadata {
        harness: Backend::Codex,
        profile_id: Some(profile.into()),
        id: context.session.clone(),
        path: temp
            .path()
            .join("session-locators/profiles")
            .join(profile)
            .join("codex-cli/existing-native"),
        project: temp.path().into(),
        title: None,
        first_user_message: None,
        parent_session: None,
        forked_from: None,
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: None,
        usage: None,
        is_running: false,
    };
    let indexed = store.update_session_metadata(&metadata)?;
    assert_eq!(
        store.register_caller_session(&context, None)?,
        indexed.app_session_id
    );
    assert_eq!(
        store.register_caller_session(&context, None)?,
        indexed.app_session_id
    );
    assert_eq!(store.cached_sessions("")?.len(), 1);
    assert_eq!(store.resolve_caller_session(&context)?.key(), indexed.key());
    store.connection.execute(
        "INSERT INTO sessions(project_id,harness,profile_id,backend_id,locator,modified_ms,created_ms) SELECT project_id,harness,profile_id,backend_id,?2,modified_ms,created_ms FROM sessions WHERE id=?1",
        params![indexed.app_session_id, temp.path().join("ambiguous.jsonl").to_string_lossy().as_ref()],
    ).map_err(|error| error.to_string())?;
    assert!(store.register_caller_session(&context, None).is_err());
    assert!(store.resolve_caller_session(&context).is_err());
    Ok(())
}

#[test]
fn caller_read_resolution_applies_scope_guards_even_with_app_id() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let profile = "11111111-1111-4111-8111-111111111111";
    let native = "profile-session";
    let metadata = crate::agents::SessionMetadata {
        harness: Backend::Codex,
        profile_id: Some(profile.into()),
        id: native.into(),
        path: temp
            .path()
            .join("session-locators/profiles")
            .join(profile)
            .join("codex-cli")
            .join(native),
        project: temp.path().into(),
        title: None,
        first_user_message: None,
        parent_session: None,
        forked_from: None,
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: None,
        usage: None,
        is_running: false,
    };
    let indexed = store.update_session_metadata(&metadata)?;
    let mut context = caller(temp.path(), native);
    context.backend = Backend::Codex;
    assert!(store.resolve_caller_session(&context).is_err());
    context.app_session_id = crate::sessions::AppSessionId::new(indexed.app_session_id);
    assert!(
        store.resolve_caller_session(&context).is_err(),
        "app hint must not bypass explicit scope"
    );
    context.harness_profile_id = Some(profile.into());
    assert_eq!(store.resolve_caller_session(&context)?.key(), indexed.key());
    context.harness_profile_id = None;
    context.session_locator = Some(indexed.path.clone());
    assert_eq!(store.resolve_caller_session(&context)?.key(), indexed.key());
    context.app_session_id = crate::sessions::AppSessionId::new(indexed.app_session_id + 1000);
    assert_eq!(
        store.resolve_caller_session(&context)?.key(),
        indexed.key(),
        "stale app hint resolves via exact locator"
    );
    let other = store.register_caller_session(&caller(temp.path(), "unrelated"), None)?;
    context.app_session_id = crate::sessions::AppSessionId::new(other);
    assert!(
        store.resolve_caller_session(&context).is_err(),
        "conflicting live hint must be rejected"
    );
    Ok(())
}

#[test]
fn native_only_default_and_named_callers_keep_distinct_app_rows() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let mut named = caller(temp.path(), "same-native");
    named.backend = Backend::Codex;
    named.harness_profile_id = Some("11111111-1111-4111-8111-111111111111".into());
    let named_id = store.register_caller_session(&named, None)?;
    let mut base = named.clone();
    base.harness_profile_id = None;
    let base_id = store.register_caller_session(&base, None)?;
    assert_ne!(base_id, named_id);
    for (caller, expected) in [(&mut base, base_id), (&mut named, named_id)] {
        assert_eq!(store.register_caller_session(caller, None)?, expected);
        assert_eq!(
            store.resolve_caller_session(caller)?.app_session_id,
            expected
        );
        caller.app_session_id = crate::sessions::AppSessionId::new(expected);
        assert_eq!(store.register_caller_session(caller, None)?, expected);
        assert_eq!(
            store.resolve_caller_session(caller)?.app_session_id,
            expected
        );
    }
    base.app_session_id = named.app_session_id;
    assert!(store.resolve_caller_session(&base).is_err());
    assert!(store.register_caller_session(&base, None).is_err());
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    assert_eq!(count, 2);
    drop(store);
    let store = StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    base.app_session_id = crate::sessions::AppSessionId::new(base_id);
    for (caller, expected) in [(&base, base_id), (&named, named_id)] {
        assert_eq!(store.register_caller_session(caller, None)?, expected);
        assert_eq!(
            store.resolve_caller_session(caller)?.app_session_id,
            expected
        );
    }
    assert_eq!(store.cached_sessions("")?.len(), 2);

    Ok(())
}
