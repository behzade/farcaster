use super::*;
use crate::agents::Backend;

#[test]
fn dismissing_trust_cancels_the_pending_project_command() {
    let mut pending = Some(RuntimeCommand::Shutdown);
    assert!(cancel_pending_command(&mut pending));
    assert!(pending.is_none());
}

#[test]
fn denying_trust_discards_a_pending_project_command() {
    let mut pending = Some(RuntimeCommand::Shutdown);
    assert!(take_trust_command(&mut pending, false).is_none());
    assert!(pending.is_none());
}

#[test]
fn selecting_after_a_new_trust_decision_restarts_the_project_process() {
    let path = PathBuf::from("/session.jsonl");
    let project = PathBuf::from("/project");
    assert!(matches!(
        restart_session_after_trust(RuntimeCommand::SelectSession {
            session_id: path.to_string_lossy().into_owned(),
            path,
            harness: Backend::Pi,
            project,
            key: None,
            request: None,
        }),
        RuntimeCommand::RestartSession { .. }
    ));
}

fn saved_chat(name: &str, project: &Path, id: i64) -> crate::sessions::SessionSummary {
    crate::sessions::SessionSummary::from_cached(
        name.into(),
        project.join(name),
        project.into(),
        name.into(),
        String::new(),
        String::new(),
        None,
        std::time::UNIX_EPOCH,
        1,
        Default::default(),
        false,
        false,
        String::new(),
    )
    .with_app_session_id(id)
}

fn select_displayed_chat(
    cx: &mut gpui::VisualTestContext,
    app: &gpui::Entity<FarcasterApp>,
    runtime: &crate::runtime::TestRuntime,
    project: &Path,
) -> crate::sessions::SessionSummary {
    let displayed = saved_chat("displayed", project, 41);
    apply(project, TrustChoice::TrustProject).unwrap();
    crate::agents::apply_project_trust(displayed.harness, project, TrustChoice::TrustProject)
        .unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.sessions.all = vec![displayed.clone()].into();
            app.sessions.visible = vec![displayed.clone()].into();
            app.select_rail_session(displayed.catalog_key(), window, cx);
        })
    });
    cx.run_until_parked();
    let request = std::iter::from_fn(|| runtime.try_recv_command())
        .find_map(|command| match command {
            RuntimeCommand::SelectSession { path, request, .. } if path == displayed.path => {
                request
            }
            _ => None,
        })
        .expect("the displayed chat reached the runtime");
    runtime.send_event(crate::runtime::RuntimeEvent::SelectionRouted(request));
    runtime.send_event(crate::runtime::RuntimeEvent::Snapshot {
        generation: 1,
        snapshot: std::sync::Arc::new(crate::runtime::RuntimeSnapshot {
            harness: Some(displayed.harness),
            project: project.into(),
            selected_session: Some(displayed.path.clone()),
            status: "Ready".into(),
            document_ready: true,
            ..Default::default()
        }),
    });
    cx.run_until_parked();
    displayed
}

fn submit_to_displayed_chat(
    app: &mut FarcasterApp,
    displayed: &crate::sessions::SessionSummary,
    runtime: &crate::runtime::TestRuntime,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) {
    let target = crate::sessions::session_target(&displayed.path);
    assert_eq!(app.composer.sessions.current_target(), target);
    assert!(app.can_submit());
    app.submit(
        "next prompt".into(),
        crate::protocol::PromptMode::Normal,
        window,
        cx,
    );
    let mut prompts = Vec::new();
    while let Some(command) = runtime.try_recv_command() {
        match command {
            RuntimeCommand::Prompt {
                target, message, ..
            } => prompts.push((target, message)),
            RuntimeCommand::NewSession { .. }
            | RuntimeCommand::ResumeDraft { .. }
            | RuntimeCommand::SelectSession { .. }
            | RuntimeCommand::RestartSession { .. } => {
                panic!("unexpected actor switch before prompt")
            }
            _ => {}
        }
    }
    assert_eq!(prompts, [(target, "next prompt".to_owned())]);
}

#[gpui::test]
fn same_project_navigation_rechecks_trust_without_blocking_the_click(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::same_project_navigation_rechecks_trust_without_blocking_the_click"
        ),
        cx,
        |cx, app, runtime, project| {
            let displayed = select_displayed_chat(cx, app, runtime, project);
            let file = trust_path().unwrap();
            let lock = file.with_extension("json.lock");
            let (latest, original_draft) = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.set_repository_project_execution(project.to_owned(), true, cx);
                    app.sessions.preferred_harness = Some(Backend::Codex);
                    while runtime.try_recv_command().is_some() {}
                    std::fs::create_dir(&lock).unwrap();
                    app.new_session(project.to_owned(), window, cx);
                    let original_draft = app.sessions.selected_draft.clone().unwrap();
                    app.composer.input.update(cx, |input, cx| {
                        input.set_value("unfinished draft", window, cx);
                    });
                    assert!(!app.overlays.view.project_trust);
                    assert!(!app.project.repository.execution_allowed);
                    let row = saved_chat("selected", project, 42);
                    app.sessions.all = vec![displayed.clone(), row.clone()].into();
                    app.sessions.visible = vec![displayed.clone(), row.clone()].into();
                    app.select_rail_session(row.catalog_key(), window, cx);
                    app.composer.input.update(cx, |input, cx| {
                        input.set_value("typed while checking trust", window, cx);
                    });
                    assert_eq!(app.sidebar_session_path(), Some(row.path.as_path()));
                    assert!(!app.overlays.view.project_trust);
                    assert!(!app.can_submit());
                    assert!(
                        !std::iter::from_fn(|| runtime.try_recv_command()).any(|c| matches!(
                            c,
                            RuntimeCommand::NewSession { .. }
                                | RuntimeCommand::SelectSession { .. }
                        ))
                    );
                    // Simulate the lock holder revoking the saved decision before it releases the lock.
                    std::fs::write(&file, "{}").unwrap();
                    std::fs::remove_dir(&lock).unwrap();
                    (row.path, original_draft)
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(!app.project.repository.execution_allowed);
                    assert!(app.overlays.view.project_trust);
                    assert!(matches!(&app.project.pending_trust_command,
                    Some(RuntimeCommand::SelectSession { path, .. }) if *path == latest));
                    app.dismiss_project_trust(window, cx);
                    assert!(app.project.pending_trust_command.is_none());
                    assert!(app.lifecycle.pending_session_switch.is_none());
                    assert!(app.sessions.selected_draft.is_none());
                    assert_eq!(
                        app.composer
                            .sessions
                            .snapshot_for(&crate::sessions::draft_target(&original_draft))
                            .text,
                        "unfinished draft"
                    );
                    assert_eq!(
                        app.composer
                            .sessions
                            .snapshot_for(&crate::sessions::session_buffer(Backend::Pi, &latest))
                            .text,
                        "typed while checking trust"
                    );
                    submit_to_displayed_chat(app, &displayed, runtime, window, cx);
                })
            });
        },
    );
}

#[gpui::test]
fn cold_draft_waits_for_its_snapshot_and_keeps_its_project_on_trust_cancel(
    cx: &mut gpui::TestAppContext,
) {
    use crate::protocol::PromptMode;
    use crate::runtime::{RuntimeEvent, RuntimeSnapshot};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::cold_draft_waits_for_its_snapshot_and_keeps_its_project_on_trust_cancel"
        ),
        cx,
        |cx, app, runtime, project| {
            let displayed = select_displayed_chat(cx, app, runtime, project);
            let other = tempfile::tempdir().unwrap();
            let text = "unfinished draft";
            let id = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.preferred_harness = Some(Backend::Codex);
                    app.new_session(project.to_owned(), window, cx);
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value(text, window, cx));
                    app.sessions.selected_draft.clone().unwrap()
                })
            });
            cx.run_until_parked();
            let sequence = std::iter::from_fn(|| runtime.try_recv_command())
                .find_map(|command| match command {
                    RuntimeCommand::NewSession {
                        id: dest, request, ..
                    } if dest == id => request,
                    _ => None,
                })
                .expect("draft navigation request");
            runtime.send_event(RuntimeEvent::SelectionRouted(sequence));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                app.drain_runtime(cx);
                assert_eq!(app.snapshot.harness, Some(displayed.harness));
                assert!(!app.can_submit());
                app.submit(text.into(), PromptMode::Normal, window, cx);
                assert!(!std::iter::from_fn(|| runtime.try_recv_command())
                    .any(|command| matches!(command, RuntimeCommand::Prompt { .. })));
                let snapshot = |harness| std::sync::Arc::new(RuntimeSnapshot {
                    harness: Some(harness), project: project.into(), ..Default::default()
                });
                runtime.send_event(RuntimeEvent::Snapshot {
                    generation: 2, snapshot: snapshot(displayed.harness),
                });
                app.drain_runtime(cx);
                assert!(!app.can_submit());
                assert_eq!(app.composer.input.read(cx).value().as_ref(), text);
                runtime.send_event(RuntimeEvent::Snapshot {
                    generation: 2, snapshot: snapshot(Backend::Codex),
                });
                app.drain_runtime(cx);
                assert!(app.can_submit());
                app.change_draft_project(other.path().into(), window, cx);
                assert!(matches!(&app.project.pending_trust_command, Some(RuntimeCommand::NewSession { project, .. }) if project == other.path()));
                app.dismiss_project_trust(window, cx);
                let original_project = crate::sessions::normalize_session_path(project);
                assert_eq!(crate::sessions::normalize_session_path(&app.project.path), original_project);
                assert_eq!(app.sessions.selected_draft.as_ref(), Some(&id));
                assert_eq!(app.composer.input.read(cx).value().as_ref(), text);
                futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                let saved = crate::app::persistence::open().unwrap().load_drafts().unwrap();
                let restored = saved.iter().find(|row| row.id == id).unwrap();
                assert_eq!(crate::sessions::normalize_session_path(&restored.project), original_project);
                assert!(app.can_submit());
                app.submit(text.into(), PromptMode::Normal, window, cx);
                let mut prompts = Vec::new();
                while let Some(command) = runtime.try_recv_command() {
                    match command {
                        RuntimeCommand::Prompt { target, message, .. } => prompts.push((target, message)),
                        RuntimeCommand::NewSession { .. } => panic!("canceled draft change reached runtime"),
                        _ => {}
                    }
                }
                assert_eq!(prompts, [(crate::sessions::draft_target(&id), text.to_owned())]);
                });
            });
        },
    );
}

#[gpui::test]
fn promoting_a_draft_during_trust_check_dispatches_its_original_selection(
    cx: &mut gpui::TestAppContext,
) {
    use crate::runtime::{RuntimeEvent, RuntimeSnapshot};
    use crate::sessions::DraftSession;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::promoting_a_draft_during_trust_check_dispatches_its_original_selection"
        ),
        cx,
        |cx, app, runtime, project| {
            let displayed = select_displayed_chat(cx, app, runtime, project);
            let saved = saved_chat("bound", project, 42);
            let mut draft =
                DraftSession::with_id(Some(saved.harness), "bound-draft".into(), project.into());
            draft.app_session_id = saved.app_session_id;
            draft.submitted = true;
            draft.session_path = Some(saved.path.clone());
            let sequence = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let rows = vec![displayed.clone(), saved.clone()];
                    app.sessions.all = rows.clone().into();
                    app.sessions.visible = rows.into();
                    app.sessions
                        .submitted_drafts
                        .insert(draft.id.clone(), draft.session_path.clone());
                    app.sessions.drafts.push(draft.clone());
                    app.resume_draft(draft.id, project.into(), window, cx);
                    let sequence = app.lifecycle.awaiting_switch_route.unwrap();
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("keep this", window, cx));
                    assert!(app.reconcile_submitted_drafts(cx));
                    assert!(app.sessions.selected_draft.is_none());
                    assert!(app.project.navigation_trust.is_some());
                    sequence
                })
            });
            cx.run_until_parked();
            let selections = std::iter::from_fn(|| runtime.try_recv_command())
                .filter_map(|command| match command {
                    RuntimeCommand::SelectSession {
                        request, path, key, ..
                    } => Some((request, path, key)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                selections,
                [(
                    Some(sequence),
                    saved.path.clone(),
                    Some(saved.catalog_key())
                )]
            );
            runtime.send_event(RuntimeEvent::SelectionRouted(sequence));
            runtime.send_event(RuntimeEvent::Snapshot {
                generation: 2,
                snapshot: std::sync::Arc::new(RuntimeSnapshot {
                    harness: Some(saved.harness),
                    selected_session: Some(saved.path.clone()),
                    project: project.into(),
                    document_ready: true,
                    ..Default::default()
                }),
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.can_submit());
                assert_eq!(app.sidebar_catalog_row(&app.sessions.all), Some(&saved));
                assert_eq!(app.composer.input.read(cx).value().as_ref(), "keep this");
            });
        },
    );
}

#[gpui::test]
fn opening_repository_trust_during_selection_restores_the_routed_chat(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::opening_repository_trust_during_selection_restores_the_routed_chat"
        ),
        cx,
        |cx, app, runtime, project| {
            let displayed = select_displayed_chat(cx, app, runtime, project);
            let requested = saved_chat("requested", project, 42);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all = vec![displayed.clone(), requested.clone()].into();
                    app.sessions.visible = vec![displayed.clone(), requested.clone()].into();
                    app.select_rail_session(requested.catalog_key(), window, cx);
                    app.composer.input.update(cx, |input, cx| {
                        input.set_value("keep in requested", window, cx)
                    });
                    app.open_project_trust(window, cx);
                    app.dismiss_project_trust(window, cx);
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(app.sidebar_session_path(), Some(displayed.path.as_path()));
                    assert_eq!(
                        app.composer
                            .sessions
                            .snapshot_for(&crate::sessions::session_buffer(
                                requested.harness,
                                &requested.path
                            ))
                            .text,
                        "keep in requested"
                    );
                    submit_to_displayed_chat(app, &displayed, runtime, window, cx);
                })
            });
        },
    );
}

#[gpui::test]
fn new_folder_draft_survives_navigation_while_database_is_busy(cx: &mut gpui::TestAppContext) {
    use std::{sync::mpsc, time::Duration};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::new_folder_draft_survives_navigation_while_database_is_busy"
        ),
        cx,
        |cx, app, runtime, project| {
            apply(project, TrustChoice::TrustProject).unwrap();
            let store = crate::app::persistence::initialize().unwrap();
            let (locked, acquired) = mpsc::channel();
            let (release, clicked) = mpsc::channel();
            let (first, second, folder) = std::thread::scope(|scope| {
                let holder = scope.spawn(move || {
                    let _store = store.lock().unwrap();
                    locked.send(()).unwrap();
                    clicked.recv_timeout(Duration::from_secs(5)).is_ok()
                });
                acquired.recv().unwrap();
                let ids = cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.sessions.preferred_harness = Some(Backend::Codex);
                        app.sessions.folders.create("Pending".into(), None);
                        let folder = app.sessions.folders.folders.last().unwrap().id;
                        app.new_session_with_folder(project.to_owned(), Some(folder), window, cx);
                        let first = app.sessions.selected_draft.clone().unwrap();
                        assert_eq!(
                            app.sessions.pending_folder_drafts.get(&first),
                            Some(&folder)
                        );
                        assert_eq!(app.sessions.draft_session_ids[&first], 0);
                        assert!(!app.can_submit());
                        app.new_session(project.to_owned(), window, cx);
                        let second = app.sessions.selected_draft.clone().unwrap();
                        assert_ne!(first, second);
                        (first, second, folder)
                    })
                });
                assert!(
                    release.send(()).is_ok() && holder.join().unwrap(),
                    "folder creation or navigation waited for the database"
                );
                ids
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                cx.run_until_parked();
                if cx.update(|_, cx| {
                    !app.read(cx)
                        .sessions
                        .pending_folder_drafts
                        .contains_key(&first)
                }) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            cx.update(|_, cx| {
                let app = app.read(cx);
                let id = app.sessions.draft_session_ids[&first];
                assert!(id > 0);
                assert_eq!(app.sessions.folders.folder_for(id), Some(folder));
                assert_eq!(
                    app.sessions.selected_draft.as_deref(),
                    Some(second.as_str())
                );
                let started = std::iter::from_fn(|| runtime.try_recv_command())
                    .filter_map(|command| match command {
                        RuntimeCommand::NewSession { id, .. } => Some(id),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(started, [second]);
            });
        },
    );
}

#[gpui::test]
fn native_actions_use_the_selected_project_while_the_snapshot_is_stale(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::native_actions_use_the_selected_project_while_the_snapshot_is_stale"
        ),
        cx,
        |cx, app, _, project| {
            let old_project = project.join("old");
            std::fs::create_dir(&old_project).unwrap();
            let row = |name: &str, parent: &Path, id| {
                crate::sessions::SessionSummary::from_cached(
                    name.into(),
                    parent.join(name),
                    parent.into(),
                    name.into(),
                    String::new(),
                    String::new(),
                    None,
                    std::time::SystemTime::UNIX_EPOCH,
                    1,
                    Default::default(),
                    false,
                    false,
                    String::new(),
                )
                .with_app_session_id(id)
            };
            let old = row("old-chat", &old_project, 11);
            let new = row("new-chat", project, 12);
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all = vec![old.clone(), new.clone()].into();
                    app.sessions.visible = vec![old.clone(), new.clone()].into();
                    app.sessions.selected_draft = None;
                    std::sync::Arc::make_mut(&mut app.snapshot).selected_session =
                        Some(old.path.clone());
                    app.select_rail_session(new.catalog_key(), window, cx);
                    assert_eq!(app.workspace_project(), project);
                    app.new_session(project.to_owned(), window, cx);
                    assert_eq!(app.workspace_project(), project);

                    app.set_repository_project_execution(project.to_owned(), true, cx);
                    app.activate_terminal_for_project(old_project.clone(), window, cx);
                    assert!(app.overlays.view.project_trust);
                    assert_eq!(app.project.trust_project.as_ref(), Some(&old_project));
                    assert!(app.workspace.terminal.view.is_none());
                    app.save_project_trust(TrustChoice::TrustProject, window, cx);
                    assert!(app.project.repository.allows_execution_in(project));
                    assert!(!app.project.repository.allows_execution_in(&old_project));
                    assert!(app.workspace.terminal.view.is_none());
                })
            });
        },
    );
}
