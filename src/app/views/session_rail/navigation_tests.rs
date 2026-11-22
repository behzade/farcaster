use super::super::session_from_row;
use super::*;
use crate::agents::Backend;
use crate::app::views::session_rail::groups::{ActiveSessionItem, session_rail_lists};
use crate::app::{AppSurface, draft_target};

fn step_matching<T>(
    active: &[T],
    archived: &[T],
    selected: impl Fn(&T) -> bool,
    direction: isize,
) -> Option<SessionStep> {
    session_step(
        active.len(),
        archived.len(),
        active.iter().chain(archived).position(selected),
        direction,
    )
}

fn archived_session(item: &ActiveSessionItem) -> &SessionSummary {
    match item {
        ActiveSessionItem::Session(item) => &item.session,
        ActiveSessionItem::Draft(_) => panic!("expected an archived chat, found a draft"),
    }
}

#[gpui::test]
fn native_navigation_restores_session_view(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(module_path!(), "::native_navigation_restores_session_view"),
        cx,
        |cx, app, _, project| {
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .expect("trust the test project");
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.drafts = [1, 2, 3]
                        .map(|id| {
                            let mut draft = crate::sessions::DraftSession::with_id(
                                Some(Backend::Pi),
                                id.to_string(),
                                project.into(),
                            );
                            draft.app_session_id = id;
                            draft.submitted = true;
                            draft
                        })
                        .to_vec();
                    app.sessions.order = vec![1, 2, 3];
                    for surface in [AppSurface::Editor, AppSurface::Terminal] {
                        app.workspace.surface = AppSurface::Chat;
                        app.sessions.selected_draft = None;
                        app.switch_composer_target(draft_target("1"), window, cx);
                        app.workspace.session_surfaces.clear();
                        app.workspace.surface = surface;
                        app.navigation.chat.focus.focus(window, cx);
                        app.sessions.selected_draft = Some("1".into());
                        app.switch_transcript_session(1, window, cx);
                        assert_eq!(app.sessions.selected_draft.as_deref(), Some("2"));
                        assert_eq!(app.workspace.surface, AppSurface::Chat);
                        assert_eq!(
                            app.workspace.session_surfaces.get(&draft_target("1")),
                            Some(&surface)
                        );
                        app.open_keybindings_help(window, cx);
                        app.overlays.sheet_focus.focus(window, cx);
                        app.switch_transcript_session(-1, window, cx);
                        assert_eq!(app.sessions.selected_draft.as_deref(), Some("2"));
                        assert!(app.overlays.sheet_focus.is_focused(window));
                        app.close_sheet(window, cx);
                        app.switch_to_session_number(3, window, cx);
                        assert_eq!(app.sessions.selected_draft.as_deref(), Some("3"));
                        assert_eq!(app.workspace.surface, AppSurface::Chat);
                        assert!(
                            !app.workspace
                                .session_surfaces
                                .contains_key(&draft_target("2"))
                        );
                    }
                });
            });
        },
    );
}

#[test]
fn worker_shortcuts_wrap_through_parent_and_workers() {
    assert_eq!(worker_step(0, Some(0), 1), None);
    assert_eq!(worker_step(1, None, 1), Some(0));
    assert_eq!(worker_step(3, Some(0), 1), Some(1));
    assert_eq!(worker_step(3, Some(0), -1), Some(2));
    assert_eq!(worker_step(3, Some(2), 1), Some(0));
}

#[test]
fn worker_shortcuts_follow_the_visible_sidebar_view() {
    let recent = visible_worker_indices(9, Some(0), false);
    let older = visible_worker_indices(9, None, true);
    assert_eq!(recent.first(), Some(&0));
    assert_eq!(older, (0..9).collect::<Vec<_>>());

    let selected_older = older[5];
    assert_eq!(
        visible_worker_indices(9, Some(selected_older), false),
        recent
            .into_iter()
            .chain([selected_older])
            .collect::<Vec<_>>()
    );
    assert_eq!(visible_worker_indices(9, Some(selected_older), true), older);
}

#[test]
fn navigation_can_leave_and_return_to_an_unsubmitted_draft() {
    use crate::app::views::session_rail::folders;
    let drafts = [(30, false), (20, true), (10, false)].map(|(id, submitted)| {
        let mut draft = crate::sessions::DraftSession::with_id(
            Some(Backend::Pi),
            format!("draft-{id}"),
            "/project".into(),
        );
        draft.app_session_id = id;
        draft.submitted = submitted;
        draft
    });
    let rows = session_rail_lists(&[], &drafts, None, &[]).active;
    let shortcuts = crate::app::views::session_rail::numbered_session_items(&rows);
    assert_eq!(
        shortcuts
            .iter()
            .map(|item| item.app_session_id())
            .collect::<Vec<_>>(),
        [20]
    );
    let ids = folders::folder_rows(rows, &Default::default())
        .into_iter()
        .filter_map(session_from_row)
        .map(|target| target.app_session_id())
        .collect::<Vec<_>>();
    assert_eq!(ids, [30, 10, 20]);
    for (selected, direction, expected) in [
        (30, 1, Some(SessionStep::Active(1))),
        (10, -1, Some(SessionStep::Active(0))),
        (20, -1, Some(SessionStep::Active(1))),
        (20, 1, Some(SessionStep::Archived(0))),
    ] {
        assert_eq!(
            step_matching(&ids, &[5], |id| *id == selected, direction),
            expected
        );
    }
}

#[test]
fn navigation_crosses_archive_preview_in_both_directions_without_wrapping() {
    let active = [10, 4];
    let archived = [30, 22, 19, 16, 15, 12, 8];
    for (selected, direction, expected) in [
        (10, -1, None),
        (10, 1, Some(SessionStep::Active(1))),
        (4, 1, Some(SessionStep::Archived(0))),
        (30, -1, Some(SessionStep::Active(1))),
        (15, 1, Some(SessionStep::Archived(5))),
        (12, 1, Some(SessionStep::Archived(6))),
        (12, -1, Some(SessionStep::Archived(4))),
        (8, 1, None),
    ] {
        assert_eq!(
            step_matching(&active, &archived, |id| *id == selected, direction),
            expected,
            "selected={selected}, direction={direction}"
        );
    }
}

#[test]
fn first_archive_expansion_highlights_the_requested_row_before_runtime_confirmation() {
    let sessions = (0..7)
        .map(|index| {
            let mut session = SessionSummary::from_cached(
                index.to_string(),
                format!("/sessions/{index}").into(),
                "/project".into(),
                format!("Session {index}"),
                String::new(),
                String::new(),
                None,
                std::time::SystemTime::UNIX_EPOCH,
                0,
                crate::sessions::UsageSummary::default(),
                true,
                false,
                String::new(),
            );
            session.app_session_id = index;
            session
        })
        .collect::<Vec<_>>();
    let sessions = SessionCatalog::from(sessions);
    let archive = session_rail_lists(&sessions, &[], None, &[]).archived;
    let previous = archived_session(&archive[0]);
    let requested = archived_session(&archive[1]);

    for confirmed in [&previous.path, &archived_session(&archive[0]).path] {
        let highlighted = selected_root(&sessions, Some(confirmed), Some(&requested.path));
        assert_eq!(
            highlighted.map(|session| session.app_session_id),
            Some(requested.app_session_id),
            "expanded archive highlighted a stale selection: {}",
            confirmed.display()
        );
    }
    let highlighted = selected_root(&sessions, Some(&requested.path), None);
    assert_eq!(
        highlighted.map(|session| session.app_session_id),
        Some(requested.app_session_id)
    );
}

#[test]
fn navigation_handles_empty_sections_and_filtered_out_selection() {
    assert_eq!(
        step_matching(&[], &[7, 3], |id| *id == 7, 1),
        Some(SessionStep::Archived(1))
    );
    assert_eq!(
        step_matching(&[7, 3], &[], |id| *id == 3, -1),
        Some(SessionStep::Active(0))
    );
    assert_eq!(step_matching(&[7], &[3], |id| *id == 99, 1), None);
    assert_eq!(step_matching(&[], &[], |id: &i64| *id == 7, 1), None);
}

fn unassigned_session(name: &str, project: &Path) -> SessionSummary {
    SessionSummary::from_cached(
        name.into(),
        crate::sessions::normalize_session_path(&project.join(name)),
        project.into(),
        name.into(),
        String::new(),
        String::new(),
        None,
        std::time::SystemTime::UNIX_EPOCH,
        0,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        String::new(),
    )
}

#[gpui::test]
fn switching_active_chats_reuses_the_expanded_archive(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::switching_active_chats_reuses_the_expanded_archive"
        ),
        cx,
        |cx, app, runtime, project| {
            use crate::runtime::RuntimeEvent;
            use gpui::{IntoElement, Render, Window};

            // Exercise both GPUI embedding paths using the actual archive view.
            struct ArchivePanel(
                gpui::Entity<crate::app::views::InactiveSessionRailView>,
                bool,
            );
            impl Render for ArchivePanel {
                fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                    crate::app::ui::accessibility::cached_panel(self.0.clone(), self.1)
                }
            }
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            let first = unassigned_session("first", project).with_app_session_id(1);
            let second = unassigned_session("second", project).with_app_session_id(2);
            let mut archived = unassigned_session("old", project).with_app_session_id(3);
            archived.archived = true;
            let parent = unassigned_session("new-parent", project).with_app_session_id(5);
            let mut orphan = unassigned_session("orphan", project).with_app_session_id(4);
            orphan.archived = true;
            orphan.parent_app_session_id = Some(parent.app_session_id);
            for accessible in [false, true] {
                let rail = cx.update(|window, cx| {
                    let rail = app.update(cx, |app, cx| {
                        let rows = vec![
                            first.clone(),
                            second.clone(),
                            archived.clone(),
                            orphan.clone(),
                        ];
                        app.sessions.all = rows.clone().into();
                        app.sessions.visible = rows.into();
                        app.sessions.drafts.clear();
                        app.sessions.selected_draft = None;
                        app.sessions.archived_expanded = true;
                        app.cancel_pending_session_switch();
                        std::sync::Arc::make_mut(&mut app.snapshot).selected_session =
                            Some(first.path.clone());
                        app.notify_session_rail(cx);
                        app.views.archived_session_rail.clone()
                    });
                    window.replace_root(cx, |_, _| ArchivePanel(rail.clone(), accessible));
                    window.draw(cx).clear(cx);
                    rail
                });
                let before = cx.update(|_, cx| rail.read(cx).preparations);
                assert!(before > 0);
                // A click across active roots, then keyboard navigation into and along the archive.
                for (target, keyboard) in [
                    (&second, false),
                    (&first, true),
                    (&orphan, true),
                    (&archived, true),
                ] {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            if keyboard {
                                app.switch_transcript_session(1, window, cx);
                            } else {
                                app.select_rail_session(target.catalog_key(), window, cx);
                            }
                            assert_eq!(
                                app.selected_rail_root().unwrap().catalog_key(),
                                target.catalog_key()
                            );
                            let mut snapshot = (*app.snapshot).clone();
                            snapshot.selected_session = Some(target.path.clone());
                            snapshot.harness = Some(target.harness);
                            snapshot.project = target.project.clone();
                            runtime.send_event(RuntimeEvent::SelectionRouted(
                                app.lifecycle
                                    .awaiting_switch_route
                                    .expect("selection request"),
                            ));
                            runtime.send_event(RuntimeEvent::Snapshot {
                                generation: app.runtime_generation,
                                snapshot: std::sync::Arc::new(snapshot),
                            });
                            app.drain_runtime(cx);
                            assert!(app.lifecycle.pending_session_switch.is_none());
                        });
                        window.draw(cx).clear(cx);
                        assert_eq!(rail.read(cx).preparations, before);
                    });
                }
                cx.update(|window, cx| {
                    let renders = rail.read(cx).renders;
                    let mut refreshed = archived.clone();
                    refreshed.model = Some(("provider".into(), "new-model".into()));
                    let rows = vec![first.clone(), second.clone(), refreshed, orphan.clone()];
                    app.update(cx, |app, cx| {
                        runtime.send_event(RuntimeEvent::Sessions {
                            generation: app.sessions.generation + 1,
                            sessions: rows.clone(),
                            all_sessions: rows,
                            activities: None,
                        });
                        app.drain_runtime(cx);
                    });
                    window.draw(cx).clear(cx);
                    assert!(
                        rail.read(cx).renders > renders,
                        "archive tooltip needs fresh row data"
                    );
                    assert_eq!(rail.read(cx).preparations, before);
                });
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        runtime.send_event(RuntimeEvent::SessionUpdated(parent.clone()));
                        app.drain_runtime(cx);
                    });
                    window.draw(cx).clear(cx);
                    assert_eq!(rail.read(cx).preparations, before + 1);
                    let rows = rail.update(cx, |rail, cx| rail.sorted_rows(app.read(cx)));
                    assert_eq!(rows.len(), 1);
                    assert!(matches!(
                        &rows[0].item,
                        super::super::ArchivedItem::Session(key) if *key == archived.catalog_key()
                    ));
                });
            }
            cx.update(|window, cx| {
                let rail = app.read(cx).views.archived_session_rail.clone();
                let before = rail.read(cx).preparations;
                app.update(cx, |app, cx| {
                    app.request_rail_archive(archived.catalog_key(), false, window, cx)
                });
                window.draw(cx).clear(cx);
                assert_eq!(rail.read(cx).preparations, before);
                assert!(
                    rail.update(cx, |rail, cx| rail.sorted_rows(app.read(cx)))
                        .is_empty()
                );
            });
        },
    );
}

#[gpui::test]
fn duplicate_unassigned_ids_step_through_each_draft_and_session(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::duplicate_unassigned_ids_step_through_each_draft_and_session"
        ),
        cx,
        |cx, app, _, project| {
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let mut rows =
                        ["a", "b", "c", "d"].map(|name| unassigned_session(name, project));
                    rows[2].archived = true;
                    rows[3].archived = true;
                    let [a, b, c, d] = rows.each_ref().map(|row| row.path.clone());
                    app.sessions.visible = rows.to_vec().into();
                    app.sessions.all = rows.to_vec().into();
                    app.sessions.drafts = ["one", "two"]
                        .map(|id| {
                            let mut draft = crate::sessions::DraftSession::with_id(
                                Some(Backend::Pi),
                                id.into(),
                                project.into(),
                            );
                            draft.app_session_id = -1;
                            draft
                        })
                        .to_vec();
                    app.sessions.selected_draft = Some("two".into());
                    app.navigation.chat.focus.focus(window, cx);
                    app.switch_relative_session(-1, window, cx);
                    assert_eq!(app.sessions.selected_draft.as_deref(), Some("one"));
                    app.switch_relative_session(1, window, cx);
                    assert_eq!(app.sessions.selected_draft.as_deref(), Some("two"));
                    app.switch_relative_session(1, window, cx);
                    assert_eq!(app.selected_rail_root().unwrap().path, a);
                    app.switch_relative_session(1, window, cx);
                    assert_eq!(app.selected_rail_root().unwrap().path, b);
                    for (direction, expected) in [(1, &c), (1, &d), (-1, &c), (-1, &b)] {
                        app.switch_transcript_session(direction, window, cx);
                        assert_eq!(&app.selected_rail_root().unwrap().path, expected);
                        assert_eq!(
                            app.sessions.archived_expanded,
                            expected == &c || expected == &d
                        );
                    }
                });
            });
        },
    );
}

#[gpui::test]
fn archive_uses_exact_unassigned_neighbor_and_rechecks_it_on_confirmation(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::archive_uses_exact_unassigned_neighbor_and_rechecks_it_on_confirmation"
        ),
        cx,
        |cx, app, runtime, project| {
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.drafts.clear();
                    app.sessions.selected_draft = None;
                    for (running, remove_neighbor) in [(false, false), (true, false), (true, true)]
                    {
                        let rows = ["a", "b", "c"].map(|name| unassigned_session(name, project));
                        app.activity.agents.clear();
                        if running {
                            let mut child = unassigned_session("working-child", project);
                            child.parent_session = Some(rows[1].id.clone());
                            let mut activity =
                                crate::agent_activity::AgentActivity::limited_fallback(&child);
                            activity.lifecycle = crate::agent_activity::AgentLifecycle::NeedsInput;
                            app.activity.agents.insert(child.activity_key(), activity);
                            let mut all = rows.to_vec();
                            all.push(child);
                            app.sessions.all = all.into();
                        } else {
                            app.sessions.all = rows.to_vec().into();
                        }
                        let b = rows[1].path.clone();
                        let c = rows[2].path.clone();
                        app.sessions.visible = rows.to_vec().into();
                        app.notify_session_rail(cx);
                        app.lifecycle.pending_session_switch = None;
                        std::sync::Arc::make_mut(&mut app.snapshot).selected_session =
                            Some(rows[0].path.clone());
                        app.select_session(b.clone(), project.into(), window, cx);
                        app.close_current_target(window, cx);
                        assert_eq!(app.sessions.pending_archive.is_some(), running);
                        if running {
                            assert_eq!(app.selected_rail_root().unwrap().path, b);
                            if remove_neighbor {
                                app.sessions.visible.retain(|row| row.path != c);
                            } else {
                                app.sessions.visible.reverse();
                            }
                            app.notify_session_rail(cx);
                            app.stop_and_archive_pending_session(window, cx);
                        }
                        let archives: Vec<_> = std::iter::from_fn(|| runtime.try_recv_command())
                            .filter_map(|command| match command {
                                crate::runtime::RuntimeCommand::SetSessionArchived {
                                    request,
                                    ..
                                } if request.archived => {
                                    let crate::runtime::ArchiveTarget::Native(target) =
                                        request.target
                                    else {
                                        panic!("an unassigned session must be archived by path");
                                    };
                                    Some((target.path, false))
                                }
                                crate::runtime::RuntimeCommand::StopSessionFamily {
                                    path, ..
                                } => Some((path, true)),
                                _ => None,
                            })
                            .collect();
                        assert_eq!(archives, [(b.clone(), running)]);
                        assert_eq!(
                            app.selected_rail_root().unwrap().path,
                            if remove_neighbor { b } else { c }
                        );
                    }
                });
            });
        },
    );
}

#[gpui::test]
fn shared_locator_keeps_the_clicked_record_through_close_and_confirmation(
    cx: &mut gpui::TestAppContext,
) {
    use crate::runtime::{ArchiveTarget, RuntimeCommand, RuntimeEvent, RuntimeSnapshot};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::shared_locator_keeps_the_clicked_record_through_close_and_confirmation"
        ),
        cx,
        |cx, app, runtime, project| {
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            crate::agents::apply_project_trust(
                Backend::Pi,
                project,
                crate::projects::TrustChoice::TrustProject,
            )
            .unwrap();
            let a = unassigned_session("a", project).with_app_session_id(11);
            let mut b = unassigned_session("b", project).with_app_session_id(22);
            b.path = a.path.clone();
            b.harness = Backend::Codex;
            let snapshot = |row: &SessionSummary| {
                std::sync::Arc::new(RuntimeSnapshot {
                    harness: Some(row.harness),
                    profile_id: row.profile_id.clone(),
                    project: row.project.clone(),
                    selected_session: Some(row.path.clone()),
                    ..Default::default()
                })
            };
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let mut child = unassigned_session("child", project).with_app_session_id(33);
                    child.parent_app_session_id = Some(11);
                    child.is_running = true;
                    let rows = vec![a.clone(), b.clone(), child];
                    app.sessions.all = rows.clone().into();
                    app.sessions.visible = rows.into();
                    app.sessions.drafts.clear();
                    app.sessions.selected_draft = None;
                    app.navigation.chat.focus.focus(window, cx);
                    app.snapshot = snapshot(&a);
                    app.switch_composer_target(
                        crate::sessions::session_buffer(a.harness, &a.path),
                        window,
                        cx,
                    );
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("first input", window, cx));
                    app.select_rail_session(b.catalog_key(), window, cx);
                    assert!(app.composer.input.read(cx).value().is_empty());
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("second input", window, cx));
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_rail_session(a.catalog_key(), window, cx);
                    assert_eq!(app.composer.input.read(cx).value().as_ref(), "first input");
                    assert_eq!(
                        app.composer
                            .sessions
                            .snapshot_for(&crate::sessions::session_buffer(b.harness, &b.path))
                            .text,
                        "second input"
                    );
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let commands = std::iter::from_fn(|| runtime.try_recv_command())
                        .filter_map(|command| match command {
                            RuntimeCommand::SelectSession { key, request, .. } => {
                                Some((key, request))
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(commands.len(), 2);
                    assert_eq!(commands[0].0, Some(b.catalog_key()));
                    assert_eq!(commands[1].0, Some(a.catalog_key()));
                    let sequence = commands[1].1.expect("superseding request");
                    for row in [&a, &b] {
                        runtime.send_event(RuntimeEvent::Snapshot {
                            generation: app.runtime_generation,
                            snapshot: snapshot(row),
                        });
                        app.drain_runtime(cx);
                        assert!(app.lifecycle.pending_session_switch.is_some());
                        assert_eq!(app.selected_rail_root(), Some(&a));
                        assert!(app.snapshot.matches_session_row(&a));
                    }
                    runtime.send_event(RuntimeEvent::SelectionRouted(sequence));
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: app.runtime_generation,
                        snapshot: snapshot(&a),
                    });
                    app.drain_runtime(cx);
                    assert!(app.lifecycle.pending_session_switch.is_none());
                    app.close_current_target(window, cx);
                    assert!(app.sessions.pending_archive.is_some());
                    app.stop_and_archive_pending_session(window, cx);
                    let stop_key =
                        std::iter::from_fn(|| runtime.try_recv_command()).find_map(|command| {
                            match command {
                                RuntimeCommand::StopSessionFamily { key, .. } => key,
                                _ => None,
                            }
                        });
                    assert_eq!(stop_key, Some(a.catalog_key()));
                    runtime.send_event(RuntimeEvent::SessionStatus {
                        record: None,
                        target: crate::sessions::AppSessionId::new(11).unwrap().to_key(),
                        session: Some(a.path.clone()),
                        status: crate::runtime::RunStatus::Working,
                    });
                    app.drain_runtime(cx);
                    assert_eq!(
                        app.rail_row_status(&a, false),
                        crate::runtime::RunStatus::Working
                    );
                    assert_eq!(
                        app.rail_row_status(&b, false),
                        crate::runtime::RunStatus::Done
                    );
                    app.request_rail_archive(b.catalog_key(), true, window, cx);
                    assert!(app.sessions.pending_archive.is_none());
                    assert!(!app.sessions.all.by_app_id(11).unwrap().archived);
                    assert!(app.sessions.all.by_app_id(22).unwrap().archived);
                    let archived =
                        std::iter::from_fn(|| runtime.try_recv_command()).find_map(|command| {
                            match command {
                                RuntimeCommand::SetSessionArchived { request, .. } => {
                                    Some(request.target)
                                }
                                _ => None,
                            }
                        });
                    assert_eq!(
                        archived,
                        Some(ArchiveTarget::App(
                            crate::sessions::AppSessionId::new(22).unwrap()
                        ))
                    );
                });
            });
        },
    );
}

#[gpui::test]
fn path_only_stop_clears_every_matching_running_row(cx: &mut gpui::TestAppContext) {
    use crate::runtime::{RunStatus, RuntimeEvent};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::path_only_stop_clears_every_matching_running_row"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let mut a = unassigned_session("a", project).with_app_session_id(11);
                    let mut b = unassigned_session("b", project).with_app_session_id(22);
                    let mut other = unassigned_session("other", project).with_app_session_id(33);
                    b.path = a.path.clone();
                    b.harness = Backend::Codex;
                    for row in [&mut a, &mut b, &mut other] {
                        row.is_running = true;
                    }
                    let rows = vec![a.clone(), b, other.clone()];
                    app.sessions.all = rows.clone().into();
                    app.sessions.visible = rows.into();
                    assert_eq!(app.sessions.all.running().count(), 3);
                    runtime.send_event(RuntimeEvent::SessionStatus {
                        record: None,
                        target: crate::sessions::session_target(&a.path),
                        session: Some(a.path),
                        status: RunStatus::Stopped,
                    });
                    app.drain_runtime(cx);
                    for catalog in [&app.sessions.all, &app.sessions.visible] {
                        assert_eq!(catalog.running().collect::<Vec<_>>(), [&other]);
                        assert!(!catalog.by_app_id(11).unwrap().is_running);
                        assert!(!catalog.by_app_id(22).unwrap().is_running);
                    }
                })
            });
        },
    );
}

#[gpui::test]
fn archiving_an_unassigned_draft_keeps_unrelated_archived_roots(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::archiving_an_unassigned_draft_keeps_unrelated_archived_roots"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let mut root = unassigned_session("unrelated", project);
                    root.archived = true;
                    let mut draft =
                        crate::sessions::DraftSession::fresh(Some(Backend::Pi), project.into());
                    draft.submitted = true;
                    let id = draft.id.clone();
                    app.sessions.drafts = vec![draft];
                    app.sessions.all = vec![root.clone()].into();
                    app.sessions.visible = vec![root.clone()].into();
                    app.request_draft_archive(id.clone(), true, window, cx);
                    let archived = super::super::groups::archived_session_rows_for_roots(
                        app.sessions.visible.roots(),
                        &app.sessions.drafts,
                        None,
                    );
                    assert_eq!(archived.len(), 2);
                    assert!(archived.iter().any(|item| match item {
                        ActiveSessionItem::Draft(draft) => draft.id == id,
                        _ => false,
                    }));
                    assert!(archived.iter().any(|item| match item {
                        ActiveSessionItem::Session(item) => item.session.path == root.path,
                        _ => false,
                    }));
                });
            });
        },
    );
}

#[gpui::test]
fn first_route_rejection_restores_the_composer_and_keeps_both_inputs(
    cx: &mut gpui::TestAppContext,
) {
    use crate::runtime::{RuntimeCommand, RuntimeEvent, RuntimeSnapshot};
    use crate::sessions::{ComposerSnapshot, session_target};
    use std::sync::Arc;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::first_route_rejection_restores_the_composer_and_keeps_both_inputs"
        ),
        cx,
        |cx, app, runtime, project| {
            let mut a = unassigned_session("displayed", project);
            let mut b = unassigned_session("requested", project);
            b.path = a.path.clone();
            b.harness = Backend::Codex;
            {
                let mut store = crate::app::persistence::open().unwrap();
                store.replace_sessions(&[a.clone(), b.clone()]).unwrap();
                let saved = store.cached_sessions("").unwrap();
                for row in [&mut a, &mut b] {
                    row.app_session_id = saved
                        .iter()
                        .find(|saved| saved.path == row.path && saved.harness == row.harness)
                        .unwrap()
                        .app_session_id;
                }
                assert_ne!(a.app_session_id, b.app_session_id);
            }
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            crate::agents::apply_project_trust(
                a.harness,
                project,
                crate::projects::TrustChoice::TrustProject,
            )
            .unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all = vec![a.clone(), b.clone()].into();
                    app.sessions.visible = vec![a.clone(), b.clone()].into();
                    app.switch_composer_target(session_target(&a.path), window, cx);
                    app.sessions.selected_draft = None;
                    app.snapshot = Arc::new(RuntimeSnapshot {
                        selected_session: Some(a.path.clone()),
                        harness: Some(a.harness),
                        project: project.into(),
                        ..Default::default()
                    });
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("A in progress", window, cx));
                    app.select_rail_session(b.catalog_key(), window, cx);
                    assert!(app.composer.input.read(cx).value().is_empty());
                    assert_eq!(
                        app.composer.sessions.current_target(),
                        session_target(&a.path)
                    );
                    assert_eq!(
                        app.composer.sessions.current_buffer(),
                        crate::sessions::session_buffer(b.harness, &b.path)
                    );
                    app.composer
                        .input
                        .update(cx, |input, cx| input.set_value("B in progress", window, cx));
                    let mut late = (*app.snapshot).clone();
                    Arc::make_mut(&mut late.conversation).push_local_user(
                        "Only A may recall this".into(),
                        0,
                        false,
                    );
                    runtime.send_event(RuntimeEvent::Snapshot {
                        generation: app.runtime_generation,
                        snapshot: Arc::new(late),
                    });
                    app.drain_runtime(cx);
                    assert!(
                        app.composer
                            .sessions
                            .previous_history(ComposerSnapshot::default())
                            .is_none()
                    );
                });
            });
            cx.run_until_parked();
            let request = std::iter::from_fn(|| runtime.try_recv_command())
                .find_map(|cmd| match cmd {
                    RuntimeCommand::SelectSession { request, path, .. } if path == b.path => {
                        request
                    }
                    _ => None,
                })
                .expect("the first switch must carry a request ID");
            runtime.send_event(RuntimeEvent::SelectionRejected(request));
            cx.run_until_parked();
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.lifecycle.pending_session_switch.is_none());
                    assert_eq!(app.sidebar_session_path(), Some(a.path.as_path()));
                    futures::executor::block_on(app.sessions.writer.flush()).unwrap();
                    assert_eq!(
                        crate::app::persistence::open()
                            .unwrap()
                            .load_selected_session(project)
                            .unwrap(),
                        Some(a.app_session_id)
                    );
                    assert_eq!(
                        app.composer.sessions.current_target(),
                        session_target(&a.path)
                    );
                    assert_eq!(
                        app.composer.input.read(cx).value().as_ref(),
                        "A in progress"
                    );
                    assert_eq!(
                        app.composer
                            .sessions
                            .snapshot_for(&crate::sessions::session_buffer(b.harness, &b.path))
                            .text,
                        "B in progress"
                    );
                    assert_eq!(
                        app.composer
                            .sessions
                            .previous_history(ComposerSnapshot::default())
                            .unwrap()
                            .text,
                        "Only A may recall this"
                    );
                })
            });
        },
    );
}

#[gpui::test]
fn switching_to_a_draft_cancels_the_persisted_selection(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::switching_to_a_draft_cancels_the_persisted_selection"
        ),
        cx,
        |cx, app, runtime, project| {
            let a = unassigned_session("first", project).with_app_session_id(11);
            let mut b = unassigned_session("second", project).with_app_session_id(22);
            b.harness = Backend::Codex;
            b.path = a.path.clone();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all = vec![a.clone(), b.clone()].into();
                    app.sessions.visible = vec![a.clone(), b.clone()].into();
                    app.sessions.selected_draft = None;
                    app.snapshot = std::sync::Arc::new(crate::runtime::RuntimeSnapshot {
                        harness: Some(a.harness),
                        project: project.into(),
                        selected_session: Some(a.path.clone()),
                        ..Default::default()
                    });
                    app.select_rail_session(b.catalog_key(), window, cx);
                    assert_eq!(app.selected_rail_root(), Some(&b));
                    app.new_session(project.into(), window, cx);
                    assert!(app.lifecycle.pending_session_switch.is_none());
                    runtime.send_event(crate::runtime::RuntimeEvent::Snapshot {
                        generation: app.runtime_generation,
                        snapshot: app.snapshot.clone(),
                    });
                    app.drain_runtime(cx);
                    assert!(app.selected_rail_root().is_none());
                    assert!(app.sidebar_catalog_row(&app.sessions.all).is_none());

                    let mut draft = crate::sessions::DraftSession::with_id(
                        Some(a.harness),
                        "bound".into(),
                        project.into(),
                    );
                    draft.app_session_id = a.app_session_id;
                    draft.submitted = true;
                    draft.session_path = Some(a.path.clone());
                    app.sessions
                        .submitted_drafts
                        .insert(draft.id.clone(), draft.session_path.clone());
                    app.sessions.drafts.push(draft.clone());
                    app.select_rail_session(b.catalog_key(), window, cx);
                    app.resume_draft(draft.id, project.into(), window, cx);
                    assert!(app.lifecycle.pending_session_switch.is_none());
                    assert!(app.selected_rail_root().is_none());
                    assert_eq!(app.sidebar_catalog_row(&app.sessions.all), Some(&a));
                });
            });
        },
    );
}
