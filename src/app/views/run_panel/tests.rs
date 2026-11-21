use super::{
    agents::{AgentSection, agent_lifecycle_visual, agent_section, lifecycle_label},
    ordered_worker_rows,
    resize::clamped_run_panel_width,
    run_panel_agent_rows, worker_navigation_rows,
};
use crate::agents::Backend;
use crate::{
    agent_activity::{AgentActivity, AgentLifecycle, AgentOutcome},
    app::ui::{assets::AppIcon, theme::theme},
};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

#[test]
fn run_panel_resize_stays_within_design_bounds() {
    assert_eq!(clamped_run_panel_width(100.0), theme().layout.run_panel_min);
    assert_eq!(clamped_run_panel_width(332.0), theme().size(332.0));
    assert_eq!(clamped_run_panel_width(500.0), theme().layout.run_panel_max);
}

#[test]
fn lifecycle_labels_and_icons_preserve_outcomes() {
    assert_eq!(
        lifecycle_label(AgentLifecycle::Completed(AgentOutcome::Failed)),
        "Failed"
    );
    assert_eq!(lifecycle_label(AgentLifecycle::Unknown), "Unknown");
    assert_eq!(
        agent_lifecycle_visual(AgentLifecycle::Completed(AgentOutcome::Complete)).0,
        AppIcon::CheckCircle
    );
    assert_eq!(
        agent_lifecycle_visual(AgentLifecycle::Completed(AgentOutcome::Incomplete)).0,
        AppIcon::WarningCircle
    );
    assert_eq!(
        agent_lifecycle_visual(AgentLifecycle::Completed(AgentOutcome::Failed)).0,
        AppIcon::XCircle
    );
}

#[test]
fn active_agents_are_never_hidden_by_limited_history() {
    assert_eq!(
        agent_section(AgentLifecycle::Working, true, true),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::NeedsInput, true, true),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::NeedsInput, true, false),
        AgentSection::Active
    );
    assert_eq!(
        agent_section(AgentLifecycle::Unknown, true, false),
        AgentSection::Limited
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            true,
            false
        ),
        AgentSection::Completed
    );
    assert_eq!(
        agent_section(
            AgentLifecycle::Completed(AgentOutcome::Complete),
            false,
            false
        ),
        AgentSection::Completed
    );
}

#[test]
fn restored_child_without_activity_gets_a_visible_selectable_fallback() {
    let session = crate::sessions::SessionSummary::from_cached(
        "child".into(),
        PathBuf::from("/sessions/child.jsonl"),
        PathBuf::from("/project"),
        "reviewer".into(),
        "Review the patch".into(),
        String::new(),
        Some("parent".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );

    let fallback = AgentActivity::limited_fallback(&session);

    assert_eq!(fallback.session_id, session.id);
    assert_eq!(fallback.session_path, session.path);
    assert_eq!(fallback.activity, "Review the patch");
    assert_eq!(
        fallback.lifecycle,
        AgentLifecycle::Completed(AgentOutcome::Complete)
    );
    assert_eq!(fallback.ended, Some(session.modified));
    assert!(fallback.limited);
    assert_eq!(
        agent_section(fallback.lifecycle, fallback.limited, session.is_running),
        AgentSection::Completed
    );
}

#[test]
fn running_child_without_activity_is_not_hidden_as_limited() {
    let session = crate::sessions::SessionSummary::from_cached(
        "child".into(),
        PathBuf::from("/sessions/child.jsonl"),
        PathBuf::from("/project"),
        "worker".into(),
        "Implement".into(),
        String::new(),
        Some("parent".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        true,
        String::new(),
    );

    let fallback = AgentActivity::limited_fallback(&session);

    assert_eq!(fallback.lifecycle, AgentLifecycle::Working);
    assert_eq!(
        agent_section(fallback.lifecycle, fallback.limited, session.is_running),
        AgentSection::Active
    );
}

#[test]
fn production_rows_include_restored_children_from_an_empty_activity_map() {
    let root = crate::sessions::SessionSummary::from_cached(
        "shared-id".into(),
        PathBuf::from("/one/root.jsonl"),
        PathBuf::from("/one"),
        "root".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    let mut child = crate::sessions::SessionSummary::from_cached(
        "shared-id".into(),
        PathBuf::from("/one/child.jsonl"),
        PathBuf::from("/one"),
        "child".into(),
        "Restored task".into(),
        String::new(),
        Some("shared-id".into()),
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    child.harness = Backend::Codex;
    let sessions = crate::sessions::SessionCatalog::from(vec![root, child.clone()]);

    let rows = run_panel_agent_rows(
        &sessions,
        &std::collections::HashMap::new(),
        sessions.by_path(Path::new("/one/root.jsonl")),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.path, child.path);
    assert_eq!(rows[0].0.session_path, child.path);
    assert_eq!(rows[0].3, AgentSection::Completed);
}

#[test]
fn worker_navigation_keeps_creation_order_when_statuses_differ() {
    let root = crate::sessions::SessionSummary::from_cached(
        "root".into(),
        "/project/root".into(),
        "/project".into(),
        "root".into(),
        String::new(),
        String::new(),
        None,
        SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    );
    let children = (0..7)
        .map(|index| {
            let mut child = crate::sessions::SessionSummary::from_cached(
                format!("child-{index}"),
                format!("/project/child-{index}").into(),
                "/project".into(),
                "worker".into(),
                String::new(),
                format!("2026-09-23T00:00:0{index}Z"),
                Some("root".into()),
                SystemTime::now(),
                0,
                crate::sessions::UsageSummary::default(),
                false,
                index == 0,
                String::new(),
            );
            child.harness = Backend::Codex;
            child
        })
        .collect::<Vec<_>>();
    let sessions = std::iter::once(root).chain(children).collect::<Vec<_>>();
    let mut limited = AgentActivity::limited_fallback(&sessions[2]);
    limited.lifecycle = AgentLifecycle::Unknown;
    let activities = std::collections::HashMap::from([(sessions[2].activity_key(), limited)]);
    let sessions = crate::sessions::SessionCatalog::from(sessions);
    let rows = worker_navigation_rows(
        &sessions,
        &activities,
        sessions.root_for_path(Some(Path::new("/project/child-3"))),
    );
    let ids = rows
        .iter()
        .map(|session| session.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "root", "child-6", "child-5", "child-4", "child-3", "child-2", "child-1", "child-0"
        ]
    );
    let ordered = ordered_worker_rows(
        &sessions,
        &activities,
        sessions.by_path(Path::new("/project/root")),
    );
    assert_eq!(ordered[5].3, AgentSection::Limited);
    assert_eq!(ordered[6].3, AgentSection::Active);
}

#[test]
fn production_rows_resolve_same_native_id_by_scoped_session_path() {
    let session = |root: &str, path: &str, parent: Option<&str>| {
        crate::sessions::SessionSummary::from_cached_for_harness(
            if parent.is_some() {
                "shared-child"
            } else {
                root
            }
            .into(),
            Backend::Codex,
            PathBuf::from(path),
            PathBuf::from(format!("/{root}")),
            root.into(),
            String::new(),
            String::new(),
            parent.map(str::to_owned),
            SystemTime::now(),
            0,
            crate::sessions::UsageSummary::default(),
            false,
            false,
            String::new(),
        )
    };
    let root_one = session("one", "/one/root", None);
    let child_one = session("one", "/one/child", Some("one"));
    let root_two = session("two", "/two/root", None);
    let child_two = session("two", "/two/child", Some("two"));
    let mut first = AgentActivity::limited_fallback(&child_one);
    first.activity = "first project".into();
    let mut second = AgentActivity::limited_fallback(&child_two);
    second.activity = "second project".into();
    let activities = std::collections::HashMap::from([
        (child_one.activity_key(), first),
        (child_two.activity_key(), second),
    ]);
    let sessions = vec![root_one, child_one, root_two, child_two.clone()];

    let sessions = crate::sessions::SessionCatalog::from(sessions);
    let rows = run_panel_agent_rows(
        &sessions,
        &activities,
        sessions.by_path(Path::new("/two/root")),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.path, child_two.path);
    assert_eq!(rows[0].0.activity, "second project");
}

#[test]
fn worker_subtitle_uses_live_profile_then_catalog_identity() {
    use super::agents::execution_label;

    let cached_model = ("cached-provider".into(), "cached-model".into());
    let profile = crate::agents::CallerProfile {
        backend: Backend::Codex,
        provider: Some("openai".into()),
        model: Some("gpt-5.6-luna".into()),
        effort: Some("high".into()),
    };
    assert_eq!(
        execution_label(Some(&profile), Some(&cached_model), Some("low")),
        "openai · gpt-5.6-luna · high"
    );
    let profile = crate::agents::CallerProfile {
        effort: None,
        ..profile
    };
    assert_eq!(
        execution_label(Some(&profile), Some(&cached_model), Some("low")),
        "openai · gpt-5.6-luna · default"
    );
    assert_eq!(
        execution_label(None, Some(&cached_model), Some("low")),
        "cached-provider · cached-model · low"
    );
    assert_eq!(execution_label(None, None, None), "Model unavailable");
    let profile = crate::agents::CallerProfile {
        model: None,
        ..profile
    };
    assert_eq!(
        execution_label(Some(&profile), Some(&cached_model), Some("low")),
        "cached-provider · cached-model · low"
    );
}

#[gpui::test]
fn catalog_generation_renders_while_the_store_is_busy(cx: &mut gpui::TestAppContext) {
    use gpui::Render as _;
    use std::{
        sync::{Arc, mpsc},
        time::{Duration, Instant},
    };

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::catalog_generation_renders_while_the_store_is_busy"
        ),
        cx,
        |cx, app, _, project| {
            // The offline harness launches each app test in its own process and data directory.
            let store = crate::app::persistence::initialize().unwrap();
            let root = project.join("root");
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let sessions = (0..=500)
                        .map(|i| {
                            let id = if i == 0 {
                                "root".to_owned()
                            } else {
                                format!("worker-{i}")
                            };
                            let path = project.join(&id);
                            let mut session = crate::sessions::SessionSummary::from_cached(
                                id.clone(),
                                path.clone(),
                                project.into(),
                                id,
                                "Measure the sidebar".into(),
                                String::new(),
                                (i > 0).then(|| "root".into()),
                                SystemTime::UNIX_EPOCH,
                                1,
                                Default::default(),
                                false,
                                false,
                                String::new(),
                            )
                            .with_app_session_id(i + 1);
                            if i > 0 {
                                session.parent_app_session_id = Some(1);
                                session.worker_profile = Some("reviewer".into());
                                app.activity
                                    .row_focus
                                    .insert(session.activity_key(), cx.focus_handle());
                            }
                            session
                        })
                        .collect::<Vec<_>>();
                    app.sessions.all = sessions.into();
                    app.sessions.selected_draft = None;
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(root);
                })
            });

            let (locked, lock_acquired) = mpsc::channel();
            let (release, rendered) = mpsc::channel();
            std::thread::scope(|scope| {
                let holder = scope.spawn(move || {
                    let _guard = store.lock().unwrap();
                    locked.send(()).unwrap();
                    // A regression can finish and report the missed signal.
                    rendered.recv_timeout(Duration::from_secs(5)).is_ok()
                });
                lock_acquired.recv().unwrap();
                let (cold, warm) = cx.update(|window, cx| {
                    let panel = app.read(cx).views.run_panel.clone();
                    panel.read(cx).invalidate_workers();
                    let render = |cx: &mut gpui::App, window: &mut gpui::Window| {
                        let start = Instant::now();
                        panel.update(cx, |panel, cx| {
                            drop(std::hint::black_box(panel.render(window, cx)))
                        });
                        start.elapsed()
                    };
                    super::worker_list::PREPARATIONS.with(|count| count.set(0));
                    super::agents::CARD_BUILDS.with(|count| count.set(0));
                    let cold = render(cx, window);
                    assert_eq!(
                        super::agents::CARD_BUILDS.with(|count| count.get()),
                        super::RECENT_WORKERS
                    );
                    let mut warm: Vec<_> = (0..50).map(|_| render(cx, window)).collect();
                    assert_eq!(
                        super::worker_list::PREPARATIONS.with(|count| count.get()),
                        1
                    );
                    warm.sort_unstable();
                    (cold, warm)
                });
                let _ = release.send(());
                assert!(
                    holder.join().unwrap(),
                    "run panel waited for the state store"
                );
                eprintln!(
                    "500 workers, store held: first panel render {cold:?}, 50 cached renders p50 {:?}, p95 {:?}",
                    warm[24], warm[47]
                );
            });
        },
    );
}

pub(super) fn worker_session(
    project: &Path,
    id: i64,
    name: &str,
    parent: Option<i64>,
) -> crate::sessions::SessionSummary {
    use crate::sessions::SessionSummary;
    let mut row = SessionSummary::from_cached(
        name.into(),
        project.join(name),
        project.to_owned(),
        name.into(),
        String::new(),
        String::new(),
        None,
        SystemTime::UNIX_EPOCH,
        0,
        Default::default(),
        false,
        false,
        String::new(),
    )
    .with_app_session_id(id);
    row.parent_app_session_id = parent;
    row
}

#[gpui::test]
fn sessions_events_only_rebuild_workers_for_a_changed_family(cx: &mut gpui::TestAppContext) {
    use crate::{agents::WorkerStatus, runtime::RuntimeEvent, sessions::SessionSummary};
    use gpui::Render as _;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::sessions_events_only_rebuild_workers_for_a_changed_family"
        ),
        cx,
        |cx, app, runtime, project| {
            let project = project.canonicalize().unwrap();
            let session = |id, name, parent| worker_session(&project, id, name, parent);
            let mut rows = vec![
                session(1, "a", None),
                session(2, "worker-a", Some(1)),
                session(3, "b", None),
                session(4, "worker-b", Some(3)),
            ];
            let key = rows[1].activity_key();
            cx.update(|window, cx| {
                app.update(cx, |app, _| {
                    app.sessions.selected_draft = None;
                    std::sync::Arc::make_mut(&mut app.snapshot).selected_session =
                        Some(rows[0].path.clone());
                });
                let mut generation = app.read(cx).sessions.generation;
                let mut publish = |cx: &mut gpui::App, rows: &[SessionSummary], index, status| {
                    generation += 1;
                    let row: &SessionSummary = &rows[index];
                    let activity = AgentActivity::from_worker_status(row, status, false);
                    runtime.send_event(RuntimeEvent::Sessions {
                        generation,
                        sessions: rows.to_vec(),
                        all_sessions: rows.to_vec(),
                        activities: Some((
                            std::collections::HashMap::from([(row.activity_key(), activity)]),
                            false,
                        )),
                    });
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                    let panel = app.read(cx).views.run_panel.clone();
                    panel.update(cx, |panel, cx| drop(panel.render(window, cx)));
                };
                super::worker_list::PREPARATIONS.with(|count| count.set(0));
                publish(cx, &rows, 1, WorkerStatus::Running);
                let focus = app.read(cx).activity.row_focus[&key].clone();
                assert_eq!(
                    super::worker_list::PREPARATIONS.with(|count| count.get()),
                    1
                );

                rows[3].title = "Other worker changed".into();
                publish(cx, &rows, 3, WorkerStatus::NeedsInput);
                assert_eq!(
                    super::worker_list::PREPARATIONS.with(|count| count.get()),
                    1
                );
                assert_eq!(app.read(cx).activity.row_focus[&key], focus);
                assert_eq!(
                    app.read(cx).activity.agents[&rows[3].activity_key()].lifecycle,
                    AgentLifecycle::NeedsInput
                );

                publish(cx, &rows, 1, WorkerStatus::Failed);
                assert_eq!(
                    super::worker_list::PREPARATIONS.with(|count| count.get()),
                    2
                );
                assert_eq!(app.read(cx).activity.row_focus[&key], focus);
                assert_eq!(
                    app.read(cx).activity.agents[&key].lifecycle,
                    AgentLifecycle::Completed(AgentOutcome::Failed)
                );

                rows[1].title = "Updated tooltip".into();
                publish(cx, &rows, 1, WorkerStatus::Failed);
                assert_eq!(
                    super::worker_list::PREPARATIONS.with(|count| count.get()),
                    3
                );
                app.update(cx, |app, _| {
                    app.activity.row_focus.remove(&key);
                });
                publish(cx, &rows, 3, WorkerStatus::NeedsInput);
                assert_ne!(app.read(cx).activity.row_focus[&key], focus);
                assert_eq!(
                    super::worker_list::PREPARATIONS.with(|count| count.get()),
                    4
                );
            });
        },
    );
}

#[gpui::test]
fn incremental_events_only_rebuild_affected_worker_families(cx: &mut gpui::TestAppContext) {
    use crate::{agents::WorkerStatus, runtime::RuntimeEvent};
    use gpui::Render as _;
    use std::sync::Arc;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::incremental_events_only_rebuild_affected_worker_families"
        ),
        cx,
        |cx, app, runtime, project| {
            let project = project.canonicalize().unwrap();
            // The selected family is an orphan until session 9 arrives.
            let root = worker_session(&project, 1, "root", Some(9));
            let worker = worker_session(&project, 2, "worker", Some(1));
            let other = worker_session(&project, 3, "other", None);
            let rows = vec![root.clone(), worker.clone()];
            cx.update(|window, cx| {
                app.update(cx, |app, _| {
                    app.sessions.selected_draft = None;
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(worker.path.clone());
                });
                let panel = app.read(cx).views.run_panel.clone();
                let generation = app.read(cx).sessions.generation + 1;
                let drain = |cx: &mut gpui::App, event| {
                    runtime.send_event(event);
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                };
                let mut publish = |cx: &mut gpui::App, event| {
                    drain(cx, event);
                    panel.update(cx, |panel, cx| drop(panel.render(window, cx)));
                    super::worker_list::PREPARATIONS.with(|count| count.get())
                };
                super::worker_list::PREPARATIONS.with(|count| count.set(0));
                assert_eq!(
                    publish(
                        cx,
                        RuntimeEvent::Sessions {
                            generation,
                            sessions: rows.clone(),
                            all_sessions: rows,
                            activities: None,
                        }
                    ),
                    1
                );
                let worker_key = worker.activity_key();
                let focus = app.read(cx).activity.row_focus[&worker_key].clone();
                assert_eq!(publish(cx, RuntimeEvent::SessionUpdated(other.clone())), 1);
                let moved = |row: &crate::sessions::SessionSummary| RuntimeEvent::SessionMoved {
                    target: row.target(),
                    target_project: project.clone(),
                    paths: Arc::new([(row.path.clone(), project.join("moved"))].into()),
                };
                let deleted =
                    |row: &crate::sessions::SessionSummary| RuntimeEvent::SessionDeleted {
                        generation,
                        paths: Arc::new([row.path.clone()].into()),
                    };
                assert_eq!(publish(cx, moved(&other)), 1);
                assert_eq!(publish(cx, deleted(&other)), 1);
                assert_eq!(app.read(cx).activity.row_focus[&worker_key], focus);

                let other_status =
                    AgentActivity::from_worker_status(&other, WorkerStatus::Running, false);
                assert_eq!(
                    publish(
                        cx,
                        RuntimeEvent::AgentActivityUpdated {
                            key: other.activity_key(),
                            activity: other_status,
                        }
                    ),
                    1
                );
                let mut activity =
                    AgentActivity::from_worker_status(&worker, WorkerStatus::NeedsInput, false);
                // Native activity retains the original locator even if its alias has
                // disappeared by the time the UI drains the event.
                activity.session_path = project.join("removed-alias").join("worker");
                assert_eq!(
                    publish(
                        cx,
                        RuntimeEvent::AgentActivityUpdated {
                            key: worker_key.clone(),
                            activity,
                        }
                    ),
                    2
                );
                assert_eq!(
                    app.read(cx).activity.agents[&worker_key].lifecycle,
                    AgentLifecycle::NeedsInput
                );

                let parent = worker_session(&project, 9, "new-parent", None);
                // Switch away before painting. The prepared orphan still needs
                // invalidation when its missing parent arrives in the background.
                let select = |cx: &mut gpui::App, path: &Path| {
                    app.update(cx, |app, _| {
                        Arc::make_mut(&mut app.snapshot).selected_session = Some(path.to_owned());
                    });
                };
                select(cx, &other.path);
                drain(cx, RuntimeEvent::SessionUpdated(parent.clone()));
                assert!(panel.read(cx).prepared_worker_root().is_none());
                select(cx, &worker.path);
                assert_eq!(publish(cx, RuntimeEvent::SessionUpdated(other.clone())), 3);
                assert!(
                    panel
                        .read(cx)
                        .has_prepared_workers_for(&parent.catalog_key())
                );
                assert_eq!(app.read(cx).activity.row_focus[&worker_key], focus);
                // A sibling move/delete in this family must still invalidate the cache.
                let sibling = worker_session(&project, 4, "sibling", Some(9));
                assert_eq!(
                    publish(cx, RuntimeEvent::SessionUpdated(sibling.clone())),
                    4
                );
                assert_eq!(publish(cx, moved(&sibling)), 5);
                assert_eq!(publish(cx, deleted(&sibling)), 6);

                let native_parent = worker_session(&project, 5, "native-parent", Some(9));
                let mut legacy = worker_session(&project, 6, "legacy-worker", None);
                legacy.parent_session = Some(native_parent.id.clone());
                assert_eq!(
                    publish(cx, RuntimeEvent::SessionUpdated(native_parent.clone())),
                    7
                );
                assert_eq!(publish(cx, RuntimeEvent::SessionUpdated(legacy.clone())), 8);
                // Both parents now have the same native ID. Catalog order gives the link
                // to the older row in the other family.
                let mut competing = other.clone();
                competing.id = native_parent.id;
                competing.modified = SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(1);
                assert_eq!(publish(cx, RuntimeEvent::SessionUpdated(competing)), 9);
                assert_eq!(
                    app.read(cx)
                        .sessions
                        .all
                        .root_for_row(&legacy)
                        .unwrap()
                        .path,
                    other.path
                );
                assert_eq!(publish(cx, RuntimeEvent::SessionUpdated(other)), 10);
                assert_eq!(
                    app.read(cx)
                        .sessions
                        .all
                        .root_for_row(&legacy)
                        .unwrap()
                        .path,
                    parent.path
                );
            });
        },
    );
}

#[gpui::test]
fn session_updates_refresh_metadata_only_worker_activity_and_parent_waiting(
    cx: &mut gpui::TestAppContext,
) {
    use crate::{
        app::session::status::roots_waiting_for_active_descendants,
        runtime::{RunStatus, RuntimeEvent},
    };
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::session_updates_refresh_metadata_only_worker_activity_and_parent_waiting"
        ),
        cx,
        |cx, app, runtime, project| {
            let root = worker_session(project, 1, "root", None);
            let mut worker = worker_session(project, 2, "worker", Some(1));
            let key = worker.activity_key();
            cx.update(|_, cx| {
                let publish = |cx: &mut gpui::App, event| {
                    runtime.send_event(event);
                    app.update(cx, |app, cx| app.drain_runtime(cx));
                };
                let state = |cx: &gpui::App| {
                    let app = app.read(cx);
                    let catalog = &app.sessions.all;
                    let rows = run_panel_agent_rows(
                        catalog,
                        &app.activity.agents,
                        catalog.by_key(&root.catalog_key()),
                    );
                    assert_eq!(rows.len(), 1);
                    assert_eq!(rows[0].2.activity_key(), key);
                    let waiting =
                        roots_waiting_for_active_descendants(catalog, &app.activity.agents);
                    (
                        rows[0].0.lifecycle,
                        app.rail_row_status(&root, waiting.contains(&root.catalog_key())),
                        app.activity.row_focus[&key].clone(),
                    )
                };
                publish(cx, RuntimeEvent::SessionUpdated(root.clone()));
                worker.is_running = true;
                publish(cx, RuntimeEvent::SessionUpdated(worker.clone()));
                let (lifecycle, parent, focus) = state(cx);
                assert_eq!(
                    (lifecycle, parent),
                    (AgentLifecycle::Working, RunStatus::Waiting)
                );

                worker.is_running = false;
                publish(cx, RuntimeEvent::SessionUpdated(worker.clone()));
                assert_eq!(
                    state(cx),
                    (
                        AgentLifecycle::Completed(AgentOutcome::Complete),
                        RunStatus::Done,
                        focus.clone()
                    )
                );

                let native = AgentActivity::from_worker_status(
                    &worker,
                    crate::agents::WorkerStatus::NeedsInput,
                    false,
                );
                publish(
                    cx,
                    RuntimeEvent::AgentActivityUpdated {
                        key: key.clone(),
                        activity: native,
                    },
                );
                worker.title = "Updated title".into();
                publish(cx, RuntimeEvent::SessionUpdated(worker));
                assert_eq!(
                    state(cx),
                    (AgentLifecycle::NeedsInput, RunStatus::Waiting, focus)
                );
            });
        },
    );
}
