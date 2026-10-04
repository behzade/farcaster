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
            RunStatus::Working,
            false
        ),
        RunStatus::Done
    );
    assert_eq!(
        resolved_session_status(
            &done,
            Some(RunStatus::Done),
            None,
            RunStatus::Invalid,
            false
        ),
        RunStatus::Done
    );
    assert_eq!(
        resolved_session_status(&running, None, None, RunStatus::Invalid, false),
        RunStatus::Working
    );
    assert_eq!(
        resolved_session_status(
            &done,
            None,
            Some(Path::new("/done.jsonl")),
            RunStatus::NeedsInput,
            false
        ),
        RunStatus::NeedsInput
    );
    assert_eq!(
        resolved_session_status(
            &running,
            Some(RunStatus::Done),
            None,
            RunStatus::Invalid,
            true
        ),
        RunStatus::Waiting
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
fn unknown_status_falls_back_and_explicit_activity_wins_over_waiting() {
    let running = session("running", None, true);
    assert_eq!(
        resolved_session_status(
            &running,
            Some(RunStatus::Invalid),
            Some(&running.path),
            RunStatus::Invalid,
            false
        ),
        RunStatus::Working
    );
    assert_eq!(
        resolved_session_status(
            &running,
            Some(RunStatus::Invalid),
            Some(&running.path),
            RunStatus::Done,
            true
        ),
        RunStatus::Waiting
    );
    assert_eq!(
        resolved_session_status(
            &running,
            Some(RunStatus::Failed),
            Some(&running.path),
            RunStatus::Working,
            true
        ),
        RunStatus::Failed
    );
}

#[gpui::test]
fn completion_badge_does_not_restart_on_repeated_done_or_survive_new_work(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::completion_badge_does_not_restart_on_repeated_done_or_survive_new_work"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let target = "session:completion".to_owned();
                    assert!(!app.record_run_status(target.clone(), RunStatus::Done, false));
                    assert!(!app.activity.run_statuses.contains_key(&target));
                    assert!(!app.record_run_status(target.clone(), RunStatus::Working, false));
                    assert!(app.record_run_status(target.clone(), RunStatus::Done, false));
                    let completed = std::time::Instant::now() - std::time::Duration::from_secs(1);
                    app.activity
                        .recent_completions
                        .insert(target.clone(), completed);
                    assert!(app.record_run_status(target.clone(), RunStatus::Done, false));
                    assert_eq!(app.activity.recent_completions[&target], completed);
                    assert!(!app.record_run_status(target.clone(), RunStatus::Working, false));
                    assert!(!app.activity.recent_completions.contains_key(&target));
                    assert_eq!(app.activity.run_statuses[&target], RunStatus::Working);
                    let alias = "session:alias".to_owned();
                    assert!(app.record_run_status(alias.clone(), RunStatus::Done, true));
                    assert!(app.activity.recent_completions.contains_key(&alias));
                    assert!(!app.record_run_status(alias.clone(), RunStatus::Failed, false));
                    assert_eq!(app.activity.run_statuses[&alias], RunStatus::Failed);
                    assert!(!app.activity.recent_completions.contains_key(&alias));
                });
            });
        },
    );
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
        resolved_session_status(&second, None, Some(&first.path), RunStatus::Working, false),
        RunStatus::Done
    );

    child.parent_app_session_id = Some(second.app_session_id);
    assert_eq!(
        roots_waiting_for_descendants(&[first, second.clone(), child]),
        HashSet::from([second.path])
    );
}
