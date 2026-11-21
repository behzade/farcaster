use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use super::*;
use crate::sessions::UsageSummary;

#[test]
fn native_status_follows_its_row_when_the_catalog_assigns_an_app_id() {
    let owner = session("native", None, false).with_app_session_id(11);
    let mut sibling = owner.clone().with_app_session_id(22);
    sibling.harness = crate::agents::Backend::Codex;
    let native = crate::agent_activity::SessionStatusRecord::Native(owner.activity_key());
    let statuses = HashMap::from([(native.status_key(), RunStatus::Working)]);

    let catalog = vec![sibling.clone()].into();
    assert_eq!(row_status(&catalog, &statuses, &sibling), None);
    let catalog = vec![owner.clone()].into();
    assert_eq!(
        row_status(&catalog, &statuses, &owner),
        Some(RunStatus::Working)
    );
}

#[gpui::test]
fn a_new_row_at_a_working_locator_keeps_its_own_badge_and_archive_decision(
    cx: &mut gpui::TestAppContext,
) {
    use crate::runtime::RuntimeEvent;
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::a_new_row_at_a_working_locator_keeps_its_own_badge_and_archive_decision"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    for (index, assigned) in [true, false].into_iter().enumerate() {
                        let mut a = session("native", None, false);
                        a.project = project.into();
                        a.path = crate::sessions::normalize_session_path(
                            &project.join(format!("shared-{index}")),
                        );
                        a.app_session_id = if assigned { 11 } else { 0 };
                        let mut b = a.clone();
                        b.app_session_id = if assigned { 22 } else { 23 };
                        b.harness = crate::agents::Backend::Codex;
                        app.sessions.all = vec![a.clone()].into();
                        app.sessions.visible = vec![a.clone()].into();
                        let alias = crate::sessions::session_target(&a.path);
                        let actor = if assigned {
                            alias.clone()
                        } else {
                            "resident-actor".into()
                        };
                        let record = crate::agent_activity::SessionStatusRecord::for_row(&a);
                        runtime.send_event(RuntimeEvent::SessionStatus {
                            record: Some(record.clone()),
                            target: actor.clone(),
                            session: Some(a.path.clone()),
                            status: RunStatus::Working,
                        });
                        app.drain_runtime(cx);
                        assert_eq!(
                            app.activity.run_statuses.get(&alias),
                            Some(&RunStatus::Working)
                        );

                        runtime.send_event(RuntimeEvent::SessionUpdated(b.clone()));
                        app.drain_runtime(cx);
                        assert_eq!(app.rail_row_status(&a, false), RunStatus::Working);
                        assert_eq!(app.rail_row_status(&b, false), RunStatus::Done);
                        for (row, expected) in [(&a, RunStatus::Working), (&b, RunStatus::Done)] {
                            let mut draft = crate::sessions::DraftSession::fresh(
                                Some(row.harness),
                                project.into(),
                            );
                            draft.session_path = Some(row.path.clone());
                            draft.app_session_id = row.app_session_id;
                            draft.submitted = true;
                            app.sessions.drafts.push(draft.clone());
                            assert_eq!(app.draft_rail_status(&draft), expected);
                            assert_eq!(app.sessions.drafts.pop(), Some(draft));
                        }
                        app.request_rail_archive(a.catalog_key(), true, window, cx);
                        assert!(app.sessions.pending_archive.is_some());
                        drop(app.close_archive_confirmation(window, cx));
                        app.request_rail_archive(b.catalog_key(), true, window, cx);
                        assert!(app.sessions.pending_archive.is_none());
                        assert!(app.sessions.all.by_key(&b.catalog_key()).unwrap().archived);

                        if !assigned {
                            a.app_session_id = 12;
                            runtime.send_event(RuntimeEvent::SessionUpdated(a.clone()));
                            app.drain_runtime(cx);
                            assert_eq!(app.rail_row_status(&a, false), RunStatus::Working);
                        }
                        runtime.send_event(RuntimeEvent::SessionStatus {
                            record: Some(record.clone()),
                            target: actor.clone(),
                            session: Some(a.path.clone()),
                            status: RunStatus::Done,
                        });
                        app.drain_runtime(cx);
                        assert_eq!(app.rail_row_status(&a, false), RunStatus::Done);
                        assert_eq!(app.rail_row_status(&b, false), RunStatus::Done);
                        assert!(!app.activity.run_statuses.contains_key(&alias));
                        assert_ne!(
                            app.activity.run_statuses.get(&actor),
                            Some(&RunStatus::Working)
                        );
                        b.is_running = true;
                        runtime.send_event(RuntimeEvent::SessionUpdated(b.clone()));
                        runtime.send_event(RuntimeEvent::SessionStatus {
                            target: actor,
                            record: Some(record),
                            session: Some(a.path.clone()),
                            status: RunStatus::Stopped,
                        });
                        app.drain_runtime(cx);
                        assert_eq!(app.rail_row_status(&a, false), RunStatus::Stopped);
                        assert!(
                            app.sessions
                                .all
                                .by_app_id(b.app_session_id)
                                .unwrap()
                                .is_running
                        );
                    }
                })
            });
        },
    );
}

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

    let waiting = roots_waiting_for_descendants(&vec![parent.clone(), child].into());

    assert!(waiting.contains(&parent.catalog_key()));
}

#[test]
fn fork_activity_does_not_change_source_status() {
    let source = session("source", None, false);
    let mut fork = session("fork", None, true);
    fork.forked_from = Some(source.id.clone());
    let catalog: SessionCatalog = vec![source.clone(), fork.clone()].into();
    let waiting = roots_waiting_for_active_descendants(&catalog, &HashMap::new());
    assert!(!waiting.contains(&source.catalog_key()));
    assert_eq!(
        resolved_session_status(
            &source,
            None,
            Some(&fork.path),
            RunStatus::Working,
            waiting.contains(&source.catalog_key())
        ),
        RunStatus::Done
    );
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
    let activities = HashMap::from([(child.activity_key(), activity)]);

    let waiting =
        roots_waiting_for_active_descendants(&vec![parent.clone(), child].into(), &activities);

    assert!(waiting.contains(&parent.catalog_key()));
}

#[cfg(unix)]
#[test]
fn activity_uses_ingested_alias_key_even_after_the_link_disappears() {
    use crate::agent_activity::AgentLifecycle;

    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    let alias = dir.path().join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let root = session("root", None, false);
    let middle = session("middle", Some("root"), false);
    let mut child = session("child", Some("middle"), false);
    child.path = real.canonicalize().unwrap().join("child.jsonl");
    let path = alias.join("child.jsonl");
    let key = crate::agent_activity::WorkerActivityKey {
        path: crate::sessions::normalize_session_path(&path),
        ..child.activity_key()
    };
    assert_eq!(key.path, child.path);
    let mut activity =
        AgentActivity::from_native_child(child.id.clone(), path, "worker", true, None);
    std::fs::remove_file(alias).unwrap();
    let expected = HashSet::from([root.catalog_key(), middle.catalog_key()]);
    let catalog = vec![root, middle, child].into();
    for lifecycle in [
        AgentLifecycle::Working,
        AgentLifecycle::NeedsInput,
        AgentLifecycle::Unknown,
    ] {
        activity.lifecycle = lifecycle;
        let activities = HashMap::from([(key.clone(), activity.clone())]);
        let waiting = roots_waiting_for_active_descendants(&catalog, &activities);
        if lifecycle == AgentLifecycle::Unknown {
            assert!(waiting.is_empty());
        } else {
            assert_eq!(waiting, expected);
        }
    }
    assert!(roots_waiting_for_descendants(&catalog).is_empty());
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
    let sessions = vec![first.clone(), second.clone(), child.clone()].into();
    assert_eq!(
        roots_waiting_for_descendants(&sessions),
        HashSet::from([first.catalog_key()])
    );
    assert_eq!(
        resolved_session_status(&second, None, Some(&first.path), RunStatus::Working, false),
        RunStatus::Done
    );

    child.parent_app_session_id = Some(second.app_session_id);
    assert_eq!(
        roots_waiting_for_descendants(&vec![first, second.clone(), child].into()),
        HashSet::from([second.catalog_key()])
    );
}
