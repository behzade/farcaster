use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use super::*;
use crate::sessions::UsageSummary;

#[test]
fn status_combines_live_catalog_and_family_activity() {
    let done = session("done", None, false);
    let running = session("running", None, true);

    assert_eq!(
        resolved_session_status(
            &done,
            None,
            Some(Path::new("/other.jsonl")),
            "Working",
            false
        ),
        "Done"
    );
    assert_eq!(
        resolved_session_status(&done, Some("Ready"), None, "", false),
        "Done"
    );
    assert_eq!(
        resolved_session_status(&running, None, None, "", false),
        "Working"
    );
    assert_eq!(
        resolved_session_status(
            &done,
            None,
            Some(Path::new("/done.jsonl")),
            "Needs input",
            false
        ),
        "Needs input"
    );
    assert_eq!(
        resolved_session_status(&running, Some("Ready"), None, "", true),
        "Waiting"
    );
}

#[test]
fn parent_waits_while_a_descendant_is_running() {
    let parent = session("parent", None, false);
    let child = session("child", Some("parent"), true);

    let waiting = roots_waiting_for_descendants(&[parent, child]);

    assert!(waiting.contains(Path::new("/parent.jsonl")));
}

#[test]
fn parent_waits_for_active_worker_when_catalog_state_is_stale() {
    let parent = session("parent", None, false);
    let child = session("child", Some("parent"), false);
    let activity = AgentActivity::from_native_child(
        child.id.clone(),
        child.path.clone(),
        "worker",
        true,
        None,
    );
    let activities = HashMap::from([(agent_activity_key(&child.path), activity)]);

    let waiting = roots_waiting_for_active_descendants(&[parent, child], &activities);

    assert!(waiting.contains(Path::new("/parent.jsonl")));
}

fn session(id: &str, parent: Option<&str>, is_running: bool) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/{id}.jsonl")),
        PathBuf::from("/project"),
        id.into(),
        String::new(),
        String::new(),
        parent.map(str::to_owned),
        if is_running {
            SystemTime::now()
        } else {
            SystemTime::UNIX_EPOCH
        },
        0,
        UsageSummary::default(),
        false,
        is_running,
        String::new(),
    )
}

#[test]
fn family_status_uses_profile_identity_and_explicit_parent_links() {
    let mut first = session("same", None, false);
    first.app_session_id = 1;
    first.profile_id = Some("first".into());
    first.path = "/first/same.jsonl".into();
    let mut second = first.clone();
    second.app_session_id = 2;
    second.profile_id = Some("second".into());
    second.path = "/second/same.jsonl".into();
    let mut child = session("child", Some("same"), true);
    child.profile_id = first.profile_id.clone();
    let sessions = [first.clone(), second.clone(), child.clone()];
    assert_eq!(
        roots_waiting_for_descendants(&sessions),
        HashSet::from([first.path.clone()])
    );
    assert_eq!(
        resolved_session_status(&second, None, Some(&first.path), "Working", false),
        "Done"
    );

    child.parent_app_session_id = Some(second.app_session_id);
    assert_eq!(
        roots_waiting_for_descendants(&[first, second.clone(), child]),
        HashSet::from([second.path])
    );
}
