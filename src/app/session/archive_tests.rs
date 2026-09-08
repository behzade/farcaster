use std::{path::PathBuf, time::SystemTime};

use super::*;
use crate::sessions::UsageSummary;

fn session(id: &str, parent: Option<&str>, archived: bool, running: bool) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/sessions/{id}.jsonl")),
        PathBuf::from("/project"),
        id.into(),
        String::new(),
        String::new(),
        parent.map(str::to_owned),
        SystemTime::now(),
        0,
        UsageSummary::default(),
        archived,
        running,
        String::new(),
    )
}

#[test]
fn active_work_includes_recursive_descendants() {
    let root = session("root", None, false, false);
    let child = session("child", Some("root"), false, false);
    let grandchild = session("grandchild", Some("child"), false, true);
    let mut sessions = [root.clone(), child, grandchild];

    assert!(session_family_has_active_work(
        &sessions,
        &root.path,
        |_| false
    ));
    sessions[2].is_running = false;
    let statuses = std::collections::HashMap::from([(
        session_target(&sessions[2].path),
        "Needs input".to_owned(),
    )]);
    let snapshot = crate::runtime::RuntimeSnapshot::default();
    let has_live_work =
        |path: &Path| super::super::activity::session_has_live_work(path, &statuses, &snapshot);
    assert!(session_family_has_active_work(
        &sessions,
        &root.path,
        has_live_work
    ));
    let unrelated = session("unrelated", None, false, false);
    assert!(!session_family_has_active_work(
        &sessions,
        &unrelated.path,
        has_live_work
    ));
}

#[test]
fn only_archived_family_events_invalidate_the_archived_rail() {
    let active = session("active", None, false, false);
    let archived = session("archived", None, true, false);
    let child = session("archived-child", Some("archived"), false, false);
    let sessions = [active.clone(), archived, child.clone()];

    assert!(!session_event_affects_archived_rail(
        &sessions,
        &session_target(&active.path),
        Some(&active.path),
    ));
    assert!(session_event_affects_archived_rail(
        &sessions,
        &session_target(&child.path),
        None,
    ));
}
