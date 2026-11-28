use crate::{app::composer::sessions::session_target, runtime::RunStatus};
use std::{path::PathBuf, time::SystemTime};

use super::*;
use crate::{
    app::{
        composer::submissions::PendingSubmission, session::activity::session_family_has_active_work,
    },
    sessions::UsageSummary,
};

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
    let mut sessions: SessionCatalog = vec![root.clone(), child, grandchild].into();

    assert!(session_family_has_active_work(
        &sessions,
        &root.path,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    ));
    sessions[2].is_running = false;
    let statuses = std::collections::HashMap::from([(
        session_target(&sessions[2].path),
        RunStatus::NeedsInput,
    )]);
    let snapshot = crate::runtime::RuntimeSnapshot::default();
    assert!(session_family_has_active_work(
        &sessions,
        &root.path,
        &statuses,
        &Default::default(),
        &snapshot,
        &Default::default(),
    ));
    let unrelated = session("unrelated", None, false, false);
    assert!(!session_family_has_active_work(
        &sessions,
        &unrelated.path,
        &statuses,
        &Default::default(),
        &snapshot,
        &Default::default(),
    ));

    let pending = std::collections::HashMap::from([(
        session_target(&sessions[1].path),
        PendingSubmission {
            id: "pending-child".into(),
            submitted_at: std::time::Instant::now(),
            submitted_target: session_target(&sessions[1].path),
            mode: crate::protocol::PromptMode::Steer,
            text: "pending child prompt".into(),
            images: Vec::new(),
            pastes: Vec::new(),
            append_on_failure: false,
            result: None,
        },
    )]);
    assert!(session_family_has_active_work(
        &sessions,
        &root.path,
        &Default::default(),
        &Default::default(),
        &crate::runtime::RuntimeSnapshot::default(),
        &pending,
    ));
}

#[test]
fn only_archived_family_events_invalidate_the_archived_rail() {
    let active = session("active", None, false, false);
    let archived = session("archived", None, true, false);
    let child = session("archived-child", Some("archived"), false, false);
    let sessions = vec![active.clone(), archived, child.clone()].into();

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

#[gpui::test]
fn archive_rollback_respects_later_clicks_and_filtered_catalogs(cx: &mut gpui::TestAppContext) {
    use crate::runtime::{RuntimeEvent, TestRuntime};

    fn next_request(runtime: &TestRuntime) -> ArchiveRequest {
        while let Some(command) = runtime.try_recv_command() {
            if let RuntimeCommand::SetSessionArchived { request, .. } = command {
                return request;
            }
        }
        panic!("missing archive intent")
    }

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::archive_rollback_respects_later_clicks_and_filtered_catalogs"
        ),
        cx,
        |cx, app, runtime, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let root = session("root", None, false, false).with_app_session_id(100);
                    let child =
                        session("child", Some("root"), false, false).with_app_session_id(101);
                    let other = session("other", None, true, false).with_app_session_id(102);
                    app.sessions.all = vec![root.clone(), child.clone(), other.clone()].into();
                    app.sessions.visible = vec![root.clone(), child.clone()].into();
                    let state = |app: &FarcasterApp, value| {
                        assert_eq!(
                            app.sessions.all.by_path(&child.path).unwrap().archived,
                            value
                        );
                        assert_eq!(
                            app.sessions.visible.by_path(&child.path).unwrap().archived,
                            value
                        );
                        assert!(!app.sessions.all.by_path(&root.path).unwrap().archived);
                        assert!(app.sessions.all.by_path(&other.path).unwrap().archived);
                        assert!(app.sessions.visible.by_path(&other.path).is_none());
                    };

                    app.request_session_archive(child.path.clone(), true, window, cx);
                    let archive = next_request(runtime);
                    state(app, true);
                    runtime.send_event(RuntimeEvent::Sessions {
                        generation: app.sessions.generation + 1,
                        sessions: vec![root.clone(), child.clone()],
                        all_sessions: vec![root.clone(), child.clone(), other.clone()],
                        activities: None,
                    });
                    app.drain_runtime(cx);
                    state(app, true);

                    app.request_session_archive(child.path.clone(), false, window, cx);
                    let restore = next_request(runtime);
                    state(app, false);
                    runtime.send_event(RuntimeEvent::SessionArchiveResult {
                        request: archive,
                        rows: Ok(vec![]),
                    });
                    app.drain_runtime(cx);
                    state(app, false);
                    runtime.send_event(RuntimeEvent::SessionArchiveResult {
                        request: restore,
                        rows: Err("fixture restore failure".into()),
                    });
                    app.drain_runtime(cx);
                    state(app, true);
                    assert_eq!(
                        app.sessions.error.as_deref(),
                        Some("fixture restore failure")
                    );

                    app.request_session_archive(child.path.clone(), false, window, cx);
                    let retry = next_request(runtime);
                    state(app, false);
                    runtime.send_event(RuntimeEvent::SessionArchiveResult {
                        request: retry,
                        rows: Ok(vec![]),
                    });
                    app.drain_runtime(cx);
                    state(app, false);
                    assert!(app.sessions.error.is_none());
                    assert!(!app.sessions.archive_writes.has_pending());
                })
            })
        },
    );
}

#[test]
fn metadata_promotes_pending_archives_and_previous_errors_with_a_placeholder_id() {
    let mut legacy = session("legacy", None, false, false);
    legacy.id = legacy.path.to_string_lossy().into_owned();
    let catalog: SessionCatalog = vec![legacy.clone()].into();
    let mut stored = legacy.clone().with_app_session_id(91);
    stored.id = "assigned-backend-id".into();
    let saved = archive_target(&stored);
    for failed in [false, true] {
        let mut writes = ArchiveWrites::default();
        let request = writes.begin(archive_target(&legacy), false, true);
        if failed {
            writes.finish(&request, false);
            writes.error = Some((request.target, "write failed".into()));
        }
        let mut incoming = stored.clone();
        assert!(writes.needs_overlay());
        writes.overlay(&mut incoming, &catalog);
        if failed {
            assert_eq!(writes.error.as_ref().unwrap().0, saved);
            assert!(!writes.needs_overlay());
            assert!(!incoming.archived);
        } else {
            assert!(incoming.archived);
            assert_eq!(writes.confirmed(&saved), Some(false));
        }
    }
}

#[test]
fn failed_archive_and_restore_keep_the_last_confirmed_value() {
    let row = session("root", None, false, false).with_app_session_id(73);
    let mut writes = ArchiveWrites::default();
    let first = writes.begin(archive_target(&row), false, true);
    let second = writes.begin(archive_target(&row), true, false);
    assert_eq!(writes.finish(&first, false), Some(false));
    assert_eq!(writes.finish(&second, false), Some(false));
    assert!(!writes.has_pending());
}

#[cfg(unix)]
#[gpui::test]
fn canonical_metadata_before_legacy_archive_receipt(cx: &mut gpui::TestAppContext) {
    use crate::{app::persistence::StateStore, runtime::RuntimeEvent};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::canonical_metadata_before_legacy_archive_receipt"
        ),
        cx,
        |cx, app, runtime, project| {
            let project = project.canonicalize().unwrap();
            let alias = project.join("alias");
            std::os::unix::fs::symlink(&project, &alias).unwrap();
            let database = project.join("archive.sqlite");
            let mut store = StateStore::open_at(&database).unwrap();
            let mut other = session("other", None, false, false);
            other.project = alias.clone();
            other.path = alias.join("other.jsonl");
            other.id = other.path.display().to_string();
            for fail in [false, true] {
                let id = if fail { "failed" } else { "saved" };
                let mut original = session(id, None, false, false);
                original.path = alias.join(format!("{id}.jsonl"));
                original.project = alias.clone();
                if fail {
                    original.id = original.path.display().to_string();
                }
                let stored = store
                    .update_session_metadata(&crate::agents::SessionMetadata {
                        harness: original.harness,
                        profile_id: None,
                        id: id.into(),
                        path: project.join(format!("{id}.jsonl")),
                        project: project.clone(),
                        title: Some("Newest metadata".into()),
                        first_user_message: None,
                        parent_session: None,
                        forked_from: None,
                        message_count: None,
                        model: None,
                        thinking_level: None,
                        service_tier: None,
                        access_mode: None,
                        usage: None,
                        is_running: false,
                    })
                    .unwrap();
                let state = |app: &FarcasterApp, archived| {
                    for catalog in [&app.sessions.all, &app.sessions.visible] {
                        assert_eq!(catalog.len(), 2);
                        assert_eq!(catalog.by_path(&other.path), Some(&other));
                        let row = catalog.by_app_id(stored.app_session_id).unwrap();
                        assert_eq!(row.path, stored.path);
                        assert_eq!(row.title, "Newest metadata");
                        assert_eq!(row.archived, archived);
                    }
                };
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.sessions.all = vec![original.clone(), other.clone()].into();
                        app.sessions.visible = vec![original.clone(), other.clone()].into();
                        app.request_session_archive(original.path.clone(), true, window, cx);
                    });
                    let request = std::iter::from_fn(|| runtime.try_recv_command())
                        .find_map(|cmd| match cmd {
                            RuntimeCommand::SetSessionArchived { request, .. } => Some(request),
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(request.target, ArchiveTarget::Native(original.target()));
                    runtime.send_event(RuntimeEvent::SessionUpdated(stored.clone()));
                    app.update(cx, |app, cx| {
                        app.drain_runtime(cx);
                        state(app, true);
                    });
                    if fail {
                        rusqlite::Connection::open(&database).unwrap().execute_batch(
                            "CREATE TRIGGER reject_archive BEFORE UPDATE OF archived_at ON sessions
                             BEGIN SELECT RAISE(ABORT, 'archive rejected'); END;"
                        ).unwrap();
                    }
                    let rows = store.archive_matching_sessions(
                        &original.path,
                        Some(&original.target()),
                        true,
                    );
                    assert_eq!(rows.is_err(), fail);
                    runtime.send_event(RuntimeEvent::SessionArchiveResult { request, rows });
                    app.update(cx, |app, cx| {
                        app.drain_runtime(cx);
                        state(app, !fail);
                        assert!(!app.sessions.archive_writes.has_pending());
                        if fail {
                            assert!(
                                app.sessions
                                    .error
                                    .as_deref()
                                    .unwrap()
                                    .contains("archive rejected")
                            );
                        }
                    });
                    let persisted = store.cached_sessions("").unwrap();
                    assert_eq!(
                        persisted
                            .iter()
                            .find(|row| row.app_session_id == stored.app_session_id)
                            .unwrap()
                            .archived,
                        !fail
                    );
                });
            }
        },
    );
}

#[gpui::test]
fn archive_result_adopts_legacy_parent_while_a_restore_is_pending(cx: &mut gpui::TestAppContext) {
    use crate::runtime::RuntimeEvent;
    use gpui::Render;
    use std::sync::Arc;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::archive_result_adopts_legacy_parent_while_a_restore_is_pending"
        ),
        cx,
        |cx, app, runtime, project| {
            let mut parent = session("legacy", None, false, false);
            let mut child = session("orphan", None, false, false).with_app_session_id(12);
            let mut other = session("other", None, false, false).with_app_session_id(13);
            for row in [&mut parent, &mut child, &mut other] {
                row.project = project.into();
                row.path = project.join(format!("{}.jsonl", row.id));
            }
            child.parent_app_session_id = Some(11);
            child.worker_profile = Some("reviewer".into());
            let mut stored = parent.clone().with_app_session_id(11);
            stored.archived = true;
            stored.title = "older persisted title".into();
            let saved = ArchiveTarget::App(AppSessionId::new(11).unwrap());
            cx.update(|window, cx| {
                app.update(cx, |app, _| {
                    app.sessions.all = vec![parent.clone(), child.clone(), other.clone()].into();
                    app.sessions.visible = vec![parent.clone(), child.clone()].into();
                    app.sessions.selected_draft = None;
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(child.path.clone());
                });
                let panel = app.read(cx).views.run_panel.clone();
                panel.update(cx, |panel, cx| drop(panel.render(window, cx)));
                assert_eq!(
                    panel.read(cx).prepared_worker_root(),
                    Some(child.catalog_key())
                );
                app.update(cx, |app, cx| {
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(other.path.clone());
                    app.request_session_archive(parent.path.clone(), true, window, cx);
                    app.request_session_archive(parent.path.clone(), false, window, cx);
                });
                let mut requests =
                    std::iter::from_fn(|| runtime.try_recv_command()).filter_map(|cmd| match cmd {
                        RuntimeCommand::SetSessionArchived { request, .. } => Some(request),
                        _ => None,
                    });
                let archive = requests.next().unwrap();
                let restore = requests.next().unwrap();
                assert_eq!(archive.target, ArchiveTarget::Native(parent.target()));
                assert_eq!(archive.target, restore.target);
                assert_eq!(
                    panel.read(cx).prepared_worker_root(),
                    Some(child.catalog_key())
                );
                runtime.send_event(RuntimeEvent::SessionArchiveResult {
                    request: archive,
                    rows: Ok(vec![stored]),
                });
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    assert_eq!(app.sessions.archive_writes.confirmed(&saved), Some(true));
                    for catalog in [&app.sessions.all, &app.sessions.visible] {
                        let current = catalog.by_app_id(11).unwrap();
                        assert_eq!(current.title, parent.title);
                        assert!(!current.archived);
                        assert_eq!(catalog.root_for_row(&child), Some(current));
                        assert_eq!(catalog.active_worker_profile(12), Some("reviewer"));
                    }
                    assert!(app.sessions.visible.by_app_id(13).is_none());
                    assert_eq!(app.sessions.all.len(), 3);
                });
                assert!(panel.read(cx).prepared_worker_root().is_none());
                runtime.send_event(RuntimeEvent::SessionArchiveResult {
                    request: restore,
                    rows: Err("restore failed".into()),
                });
                app.update(cx, |app, cx| {
                    app.drain_runtime(cx);
                    for catalog in [&app.sessions.all, &app.sessions.visible] {
                        assert!(catalog.by_app_id(11).unwrap().archived);
                        assert_eq!(catalog.active_worker_profile(12), None);
                    }
                    assert!(!app.sessions.archive_writes.has_pending());
                });
            });
        },
    );
}
