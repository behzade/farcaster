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
        session("root", "/locators/profiles/a/codex-cli/root", None),
        session(
            "child",
            "/locators/profiles/a/codex-cli/child",
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
