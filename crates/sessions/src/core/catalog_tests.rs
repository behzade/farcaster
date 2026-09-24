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
        session("root", "/locators/profiles/a/codex-cli/root", None),
        session(
            "child",
            "/locators/profiles/a/codex-cli/child",
            Some("root"),
        ),
    ];
    let built_in =
        session_family_for_path(&sessions, Path::new("/locators/codex-cli/root")).unwrap();
    assert_eq!(built_in.len(), 1);
    let custom =
        session_family_for_path(&sessions, Path::new("/locators/profiles/a/codex-cli/root"))
            .unwrap();
    assert_eq!(custom.len(), 2);
    assert_eq!(custom[1].id, "child");
}
