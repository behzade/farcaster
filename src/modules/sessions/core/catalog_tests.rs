use super::*;

#[test]
fn imported_orphan_child_is_exposed_as_a_root() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let mut store =
        crate::app::persistence::StateStore::open_at(&temp.path().join("state.sqlite3"))?;
    let unrelated_parent = SessionSummary::from_cached_for_harness(
        "not-imported-parent".into(),
        "pi".into(),
        temp.path().join("session-locators/pi/not-imported-parent"),
        temp.path().to_path_buf(),
        "Same native ID, other backend".into(),
        "unrelated".into(),
        String::new(),
        None,
        std::time::SystemTime::now(),
        1,
        Default::default(),
        false,
        false,
        "unrelated".into(),
    );
    store.index_sessions(&[unrelated_parent], false)?;
    let child = SessionSummary::from_cached_for_harness(
        "child".into(),
        "codex-cli".into(),
        temp.path().join("session-locators/codex-cli/child"),
        temp.path().to_path_buf(),
        "Imported child".into(),
        "work".into(),
        String::new(),
        Some("not-imported-parent".into()),
        std::time::SystemTime::now(),
        1,
        Default::default(),
        false,
        false,
        "work".into(),
    );
    store.index_sessions(&[child], false)?;

    let cached = store.cached_sessions("")?;
    assert_eq!(cached.len(), 2);
    let roots = root_sessions(&cached);
    assert_eq!(roots.len(), 2);
    assert!(roots.iter().any(|session| session.id == "child"));

    let parent = SessionSummary::from_cached_for_harness(
        "not-imported-parent".into(),
        "codex-cli".into(),
        temp.path()
            .join("session-locators/codex-cli/not-imported-parent"),
        temp.path().to_path_buf(),
        "Imported parent".into(),
        "parent work".into(),
        String::new(),
        None,
        std::time::SystemTime::now(),
        1,
        Default::default(),
        false,
        false,
        "parent work".into(),
    );
    store.index_sessions(&[parent], false)?;
    let cached = store.cached_sessions("")?;
    let roots = root_sessions(&cached);
    assert_eq!(cached.len(), 3);
    assert_eq!(roots.len(), 2);
    assert!(roots.iter().any(|session| {
        session.id == "not-imported-parent"
            && session.project == crate::sessions::normalize_session_path(temp.path())
    }));
    let imported_parent = roots
        .iter()
        .find(|session| session.harness == "codex-cli")
        .ok_or("missing same-backend parent")?;
    assert_eq!(
        descendant_sessions_for_root(&cached, imported_parent).len(),
        1
    );
    assert_eq!(
        descendant_sessions_for_root(&cached, imported_parent)[0]
            .0
            .id,
        "child"
    );
    store.save_worker_family(&crate::agents::WorkerFamilyLink {
        project: temp.path().to_path_buf(),
        parent_backend: "codex-cli".into(),
        parent_session: "not-imported-parent".into(),
        child_backend: "opencode2".into(),
        child_session: "cross-backend-child".into(),
        execution: None,
    })?;
    let cached = store.cached_sessions("")?;
    let cross_backend_child = cached
        .iter()
        .find(|session| session.id == "cross-backend-child")
        .ok_or("missing explicit cross-backend child")?;
    assert_eq!(
        cross_backend_child.parent_harness.as_deref(),
        Some("codex-cli")
    );
    assert!(
        !root_sessions(&cached)
            .iter()
            .any(|session| session.id == "cross-backend-child")
    );
    Ok(())
}
