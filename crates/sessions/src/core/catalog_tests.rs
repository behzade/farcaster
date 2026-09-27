use super::*;
use crate::{SessionImport, UsageSummary};
use farcaster_contracts::Backend;
use std::{path::PathBuf, time::SystemTime};

fn session(id: &str, path: &str, parent: Option<&str>) -> SessionSummary {
    SessionSummary::import(SessionImport {
        id: id.into(),
        harness: Backend::Codex,
        path: PathBuf::from(path),
        project: "/project".into(),
        title: id.into(),
        first_user_message: String::new(),
        timestamp: String::new(),
        parent_session: parent.map(str::to_owned),
        modified: SystemTime::now(),
        message_count: 0,
        usage: UsageSummary::default(),
        archived: false,
        is_running: false,
        search: String::new(),
    })
}

#[test]
fn same_native_id_in_two_profiles_has_separate_families() {
    let sessions = vec![
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
    ];
    let built_in =
        session_family_for_path(&sessions, Path::new("/locators/codex-cli/root")).unwrap();
    assert_eq!(built_in.len(), 1);
    let custom = session_family_for_path(
        &sessions,
        Path::new("/locators/profiles/11111111-1111-4111-8111-111111111111/codex-cli/root"),
    )
    .unwrap();
    assert_eq!(custom.len(), 2);
    assert_eq!(custom[1].id, "child");
}

#[test]
fn archived_family_guard_treats_unresolved_parents_as_roots() {
    let mut native_orphan = session("native-orphan", "/native-orphan", Some("missing"));
    native_orphan.archived = true;
    let mut app_orphan = session("app-orphan", "/app-orphan", Some("homonym"));
    app_orphan.archived = true;
    app_orphan.parent_app_session_id = Some(999);
    let mut homonym = session("homonym", "/homonym", None);
    homonym.app_session_id = 1;
    let rows = vec![native_orphan, app_orphan, homonym];
    for orphan in &rows[..2] {
        assert_eq!(
            archived_root_family_for_path(&rows, &orphan.path),
            Some(vec![orphan])
        );
    }
}

#[test]
fn archived_family_guard_rejects_a_resolved_application_parent_without_native_parent() {
    let mut root = session("root", "/root", None);
    root.app_session_id = 1;
    root.archived = true;
    let mut child = session("child", "/child", None);
    child.parent_app_session_id = Some(1);
    child.archived = true;
    let rows = vec![root, child];
    assert!(archived_root_family_for_path(&rows, &rows[1].path).is_none());
    assert_eq!(
        archived_root_family_for_path(&rows, &rows[0].path),
        Some(vec![&rows[0], &rows[1]])
    );
}
