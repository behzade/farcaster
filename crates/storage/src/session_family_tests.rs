use super::*;
use crate::agents::Backend;

#[test]
fn persisted_cross_project_parent_survives_catalog_refresh_and_family_queries() -> Result<(), String>
{
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    // Match persistence's canonical paths, including macOS /var -> /private/var.
    let root = temp
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let database = root.join("state.sqlite3");
    let mut store = StateStore::open_at(&database)?;
    let make_session = |project: &str, id: &str, parent: Option<&str>| {
        SessionSummary::from_cached_for_harness(
            id.into(),
            Backend::Codex,
            root.join(project).join(id),
            root.join(project),
            id.into(),
            id.into(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::now(),
            1,
            Default::default(),
            false,
            false,
            id.into(),
        )
    };
    let parent = make_session("parent-project", "parent", None);
    let child = make_session("child-project", "child", Some("parent"));
    let homonym = make_session("child-project", "parent", None);
    let imported = [parent.clone(), child.clone(), homonym.clone()];
    store.index_sessions(&imported, false)?;
    // Existing databases can contain a resolved cross-project link. Do not
    // replace it with a same-native-ID session in the child's project.
    store.connection.execute(
        "UPDATE sessions SET parent_id=(SELECT id FROM sessions WHERE locator=?1) WHERE locator=?2",
        params![parent.path.to_string_lossy(), child.path.to_string_lossy()],
    ).map_err(|error| error.to_string())?;
    store.set_session_archived(&parent.path, true)?;
    drop(store);
    let mut store = StateStore::open_at(&database)?;
    let assert_parent = |store: &StateStore| {
        let cached = store.cached_sessions("").expect("load cached sessions");
        let root = crate::sessions::root_session_for_path(&cached, Some(&child.path))
            .expect("child family root");
        assert_eq!(root.path, parent.path);
        assert!(root.archived);
    };
    assert_parent(&store);
    store.index_sessions(&imported, false)?;
    assert_parent(&store);
    store.update_session_metadata(&crate::agents::SessionMetadata {
        harness: child.harness,
        id: child.id.clone(),
        path: child.path.clone(),
        project: child.project.clone(),
        title: None,
        first_user_message: None,
        parent_session: child.parent_session.clone(),
        message_count: None,
        model: None,
        thinking_level: None,
        service_tier: None,
        access_mode: None,
        usage: None,
        is_running: false,
    })?;
    assert_parent(&store);
    let cached = store.cached_sessions("")?;
    assert!(
        !crate::sessions::root_sessions(&cached)
            .iter()
            .any(|s| s.path == child.path)
    );
    let family =
        crate::sessions::session_family_for_path(&cached, &parent.path).expect("parent family");
    assert_eq!(
        family.iter().map(|s| &s.path).collect::<Vec<_>>(),
        [&parent.path, &child.path]
    );
    let other_family =
        crate::sessions::session_family_for_path(&cached, &homonym.path).expect("homonym family");
    assert_eq!(other_family.len(), 1);
    let filtered = store.cached_sessions("child")?;
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|s| s.path == parent.path));
    assert!(!filtered.iter().any(|s| s.path == homonym.path));
    let without_parent = cached
        .into_iter()
        .filter(|s| s.path != parent.path)
        .collect::<Vec<_>>();
    assert_eq!(
        crate::sessions::root_session_for_path(&without_parent, Some(&child.path))
            .expect("orphan child root")
            .path,
        child.path,
        "a missing resolved parent must not bind to a native-ID homonym"
    );
    Ok(())
}
