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
    // A cached but unresolved parent must not fall back to its native-ID homonym.
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
    // Reordering changes every stored position; editing changes identity and metadata.
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
