use super::*;
use crate::{SessionImport, UsageSummary, descendant_sessions_for_root, root_session_for_path};
use farcaster_contracts::Backend;
use std::time::SystemTime;

fn session(id: &str, path: &str, parent: Option<&str>) -> SessionSummary {
    SessionSummary::import(SessionImport {
        id: id.into(),
        harness: Backend::Codex,
        path: path.into(),
        project: "/project".into(),
        title: id.into(),
        first_user_message: String::new(),
        timestamp: String::new(),
        parent_session: parent.map(str::to_owned),
        forked_from: None,
        modified: SystemTime::UNIX_EPOCH,
        message_count: 0,
        usage: UsageSummary::default(),
        archived: false,
        is_running: false,
        search: String::new(),
    })
}

#[test]
fn cached_relationships_preserve_profile_identity_orphans_and_cycles() {
    let mut rows = vec![
        session("root", "/locators/codex-cli/root", None),
        session(
            "root",
            "/locators/profiles/11111111-1111-4111-8111-111111111111/codex-cli/root",
            None,
        ),
        session(
            "child",
            "/locators/profiles/11111111-1111-4111-8111-111111111111/codex-cli/child",
            Some("root"),
        ),
        session("orphan", "/locators/codex-cli/orphan", Some("missing")),
        session("cycle-a", "/locators/codex-cli/cycle-a", Some("cycle-b")),
        session("cycle-b", "/locators/codex-cli/cycle-b", Some("cycle-a")),
    ];
    rows[3].parent_session = Some("root".into());
    rows[3].parent_app_session_id = Some(999);
    let catalog = SessionCatalog::from(rows);
    for row in catalog.iter() {
        let expected = root_session_for_path(&catalog, Some(&row.path)).unwrap();
        assert_eq!(
            catalog.root_for_path(Some(&row.path)).unwrap().path,
            expected.path
        );
        let paths = |rows: Vec<(&SessionSummary, usize)>| {
            rows.into_iter()
                .map(|(row, depth)| (row.path.clone(), depth))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(catalog.descendants(row)),
            paths(descendant_sessions_for_root(&catalog, row))
        );
    }
    assert_eq!(
        catalog.roots().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["root", "root", "orphan"]
    );
    assert!(catalog.root_for_path(None).is_none());
}

#[test]
fn mutation_rebuilds_relationships_and_returns_current_metadata() {
    let mut catalog = SessionCatalog::from(vec![
        session("root", "/root", None),
        session("child", "/child", Some("root")),
    ]);
    assert_eq!(
        catalog.root_for_path(Some(Path::new("/child"))).unwrap().id,
        "root"
    );
    catalog.reverse();
    catalog[1].title = "Renamed".into();
    catalog[1].path = "/moved".into();
    catalog[1].usage.total = 100;
    let root = catalog.root_for_path(Some(Path::new("/child"))).unwrap();
    assert_eq!(root.path, Path::new("/moved"));
    assert_eq!(root.title, "Renamed");
    assert_eq!(root.usage.total, 100);
    assert!(catalog.root_for_path(Some(Path::new("/root"))).is_none());
    catalog.retain(|row| row.id != "root");
    assert_eq!(
        catalog.root_for_path(Some(Path::new("/child"))).unwrap().id,
        "child"
    );
    catalog.push(session("root", "/new-root", None));
    assert_eq!(
        catalog
            .root_for_path(Some(Path::new("/child")))
            .unwrap()
            .path,
        Path::new("/new-root")
    );
}

#[test]
fn family_queries_use_explicit_profiles_and_canonical_cross_project_parents() {
    let mut builtin = session("root", "/native/builtin.jsonl", None);
    builtin.app_session_id = 1;
    let mut custom = session("root", "/native/custom.jsonl", None);
    custom.app_session_id = 2;
    custom.profile_id = Some("custom".into());
    let mut child = session("child", "/native/child.jsonl", Some("root"));
    child.app_session_id = 3;
    child.profile_id = Some("custom".into());
    let mut grandchild = session("grandchild", "/native/grandchild.jsonl", None);
    grandchild.parent_app_session_id = Some(3);
    grandchild.archived = true;
    grandchild.project = "/other-project".into();
    grandchild.harness = Backend::Pi;
    let catalog = SessionCatalog::from(vec![builtin, custom, child, grandchild]);
    let index = SessionRootIndex::new(&catalog);
    assert_eq!(catalog.child_count(&catalog[0]), 0);
    assert_eq!(catalog.child_count(&catalog[1]), 1);
    assert_eq!(catalog.child_count(&catalog[2]), 1);
    assert_eq!(catalog.child_count(&catalog[3]), 0);
    for row in catalog.iter() {
        assert_eq!(catalog.ancestors(row), index.ancestors(row));
    }
    let ancestors = catalog.ancestors(&catalog[3]);
    assert_eq!(ancestors, [&catalog[2], &catalog[1]]);
    assert_eq!(index.parent(&catalog[2]), Some(&catalog[1]));
    assert!(crate::is_subagent_path(&catalog, &catalog[3].path));
    assert!(crate::archived_root_family_for_path(&catalog, &catalog[3].path).is_none());
    assert_eq!(
        catalog.root_for_path(Some(&catalog[3].path)),
        Some(&catalog[1])
    );
}

#[test]
fn ancestor_queries_exclude_self_in_cycles_and_invalidate_after_mutation() {
    let mut catalog = SessionCatalog::from(vec![
        session("a", "/a", Some("b")),
        session("b", "/b", Some("a")),
        session("self", "/self", Some("self")),
    ]);
    for row in catalog.iter() {
        let ancestors = catalog.ancestors(row);
        assert_eq!(ancestors, SessionRootIndex::new(&catalog).ancestors(row));
        assert!(ancestors.iter().all(|ancestor| ancestor.path != row.path));
    }
    assert_eq!(catalog.ancestors(&catalog[0]), [&catalog[1]]);
    assert!(catalog.ancestors(&catalog[2]).is_empty());
    catalog[0].parent_session = None;
    assert!(catalog.ancestors(&catalog[0]).is_empty());
    assert_eq!(catalog.child_count(&catalog[1]), 0);
    assert_eq!(catalog.ancestors(&catalog[1]), [&catalog[0]]);
}

#[test]
fn saved_worker_profile_tracks_both_archive_flags_by_record_id() {
    let parent = session("parent", "/shared", None).with_app_session_id(1);
    let mut child = session("child", "/child", None).with_app_session_id(2);
    child.parent_app_session_id = Some(1);
    child.worker_profile = Some("reviewer".into());
    let mut other = session("other", "/shared", None).with_app_session_id(3);
    other.archived = true;
    let mut catalog = SessionCatalog::from(vec![parent, child, other]);
    assert_eq!(catalog.active_worker_profile(2), Some("reviewer"));
    catalog[0].archived = true;
    assert_eq!(catalog.active_worker_profile(2), None);
    catalog[0].archived = false;
    catalog[1].archived = true;
    assert_eq!(catalog.active_worker_profile(2), None);
    catalog[1].archived = false;
    assert_eq!(catalog.active_worker_profile(2), Some("reviewer"));
}

#[test]
fn archive_and_stop_keep_warm_relationships_and_unrelated_rows() {
    let root = session("root", "/root", None).with_app_session_id(10);
    let mut child = session("child", "/child", Some("root")).with_app_session_id(11);
    child.is_running = true;
    let mut catalog = SessionCatalog::from(vec![root, child]);
    assert_eq!(
        catalog
            .running()
            .map(|row| row.app_session_id)
            .collect::<Vec<_>>(),
        [11]
    );
    assert_eq!(catalog.by_app_id(11).unwrap().path, Path::new("/child"));
    assert!(catalog.set_row_archived(&catalog[0].target(), 10, true));
    assert!(catalog.relationships.get().is_some());
    catalog.mark_stopped(Path::new("/child"));
    assert!(catalog.relationships.get().is_some());
    assert!(
        catalog
            .root_for_path(Some(Path::new("/child")))
            .unwrap()
            .archived
    );
    assert!(!catalog.by_app_id(11).unwrap().archived);
    assert!(!catalog.by_app_id(11).unwrap().is_running);
    assert!(catalog.running().next().is_none());
    catalog[0].is_running = true;
    assert_eq!(catalog.running().next(), Some(&catalog[0]));
    assert!(!catalog.set_row_archived(&catalog[0].target(), 77, false));
    assert!(catalog.by_app_id(10).unwrap().archived);
}

#[cfg(unix)]
#[test]
fn archive_receipt_reconciles_only_the_requested_legacy_placeholder() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let alias = project.join("alias");
    std::os::unix::fs::symlink(&project, &alias).unwrap();
    let placeholder = |name: &str| {
        let path = alias.join(name);
        let mut row = session(path.to_str().unwrap(), path.to_str().unwrap(), None);
        row.project = alias.clone();
        row
    };
    let requested = placeholder("first");
    let other = placeholder("second");
    let mut metadata = requested.clone().with_app_session_id(81);
    metadata.project = project.clone();
    metadata.path = project.join("first");
    metadata.id = "native-id".into();
    metadata.title = "Current title".into();
    let mut receipt = metadata.clone();
    receipt.title = "Older title".into();
    receipt.archived = true;

    let mut catalog = SessionCatalog::from(vec![requested.clone(), other.clone()]);
    assert!(catalog.upsert_in_place(metadata.clone()));
    assert_eq!(catalog.len(), 3);
    let i = catalog
        .apply_archive_result(&receipt, Some(&requested.path), true)
        .unwrap();
    metadata.archived = true;
    assert_eq!(catalog[i], metadata);
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog.by_app_id(81), Some(&metadata));
    assert_eq!(catalog.by_path(&other.path), Some(&other));
    assert!(catalog.by_path(&requested.path).is_none());
}

#[test]
fn cached_counts_and_ancestors_distinguish_stored_rows_with_the_same_locator() {
    let first = session("first", "/shared", None).with_app_session_id(1);
    let second = session("second", "/shared", None).with_app_session_id(2);
    let mut child = session("child", "/child", None).with_app_session_id(3);
    child.parent_app_session_id = Some(1);
    let catalog = SessionCatalog::from(vec![first, second, child]);
    assert_eq!(catalog.child_count(&catalog[0]), 1);
    assert_eq!(catalog.child_count(&catalog[1]), 0);
    assert_eq!(catalog.ancestors(&catalog[2]), [&catalog[0]]);
    let absent = catalog[0].clone().with_app_session_id(99);
    assert_eq!(catalog.child_count(&absent), 0);
}
