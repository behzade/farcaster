use super::*;
use gpui::AppContext as _;

struct ArchiveListHarness {
    scroll: UniformListScrollHandle,
    count: usize,
    selected: Option<usize>,
}

impl Render for ArchiveListHarness {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::{InteractiveElement as _, Styled as _, uniform_list};
        let height = session_row_height(false);
        let selected = self.selected;
        uniform_list("archive-test", self.count, move |range, _, _| {
            range
                .map(|index| {
                    gpui::div().h(height).w_full().debug_selector(move || {
                        if Some(index) == selected {
                            "archive-selected-row".into()
                        } else {
                            format!("archive-row-{index}").into()
                        }
                    })
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .size_full()
    }
}

#[gpui::test]
fn transcript_redraws_reuse_sidebar_until_search_changes(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::transcript_redraws_reuse_sidebar_until_search_changes"
        ),
        cx,
        |cx, app, _, _| {
            let (rail, transcript, search) = cx.update(|_, cx| {
                let app = app.read(cx);
                (
                    app.views.session_rail.clone(),
                    app.views.transcript.clone(),
                    app.navigation.search.clone(),
                )
            });
            let mut previous_renders = 0;
            for query in ["", "changed search"] {
                cx.update(|window, cx| {
                    search.update(cx, |search, cx| search.set_value(query, window, cx));
                });
                for _ in 0..3 {
                    cx.update(|window, cx| window.draw(cx).clear(cx));
                }
                let renders = cx.update(|_, cx| rail.read(cx).render_count);
                assert!(
                    renders > previous_renders,
                    "search edits must redraw the sidebar"
                );
                for _ in 0..3 {
                    cx.update(|_, cx| transcript.update(cx, |_, cx| cx.notify()));
                    cx.update(|window, cx| window.draw(cx).clear(cx));
                }
                assert_eq!(cx.update(|_, cx| rail.read(cx).render_count), renders);
                previous_renders = renders;
            }
        },
    );
}

#[gpui::test]
fn first_archive_expansion_does_not_scroll_past_the_selected_row(cx: &mut gpui::TestAppContext) {
    use gpui::{point, px, size};
    let cx = cx.add_empty_window();
    let list = UniformListScrollHandle::new();
    let rows: Vec<_> = (0..12).map(|index| format!("session:{index}")).collect();
    let height = session_row_height(false);
    for selected in [5, 4] {
        let mut reveal = Some(SessionReveal::SessionID(format!("session:{selected}")));
        reveal_archived_session_row(
            &list,
            rows.len(),
            |id| rows.iter().position(|row| row == id),
            &mut reveal,
        );
        cx.draw(
            point(px(0.), px(0.)),
            size(px(120.), height * 2.),
            |_, cx| {
                cx.new(|_| ArchiveListHarness {
                    scroll: list.clone(),
                    count: 12,
                    selected: Some(selected),
                })
                .into_any_element()
            },
        );
        let bounds = cx
            .debug_bounds("archive-selected-row")
            .expect("selected archived row");
        assert!(bounds.top() >= px(0.));
        assert!(bounds.bottom() <= height * 2.);
        assert!(reveal.is_none());
    }
}

#[gpui::test]
fn archive_content_size_tracks_insertions_and_viewport_resizes(cx: &mut gpui::TestAppContext) {
    use gpui::{point, px, size};
    let cx = cx.add_empty_window();
    let list = UniformListScrollHandle::new();
    let height = session_row_height(false);
    for (count, width) in [(4_000, 120.), (4_001, 120.), (4_001, 240.)] {
        cx.draw(
            point(px(0.), px(0.)),
            size(px(width), height * 5.),
            |_, cx| {
                cx.new(|_| ArchiveListHarness {
                    scroll: list.clone(),
                    count,
                    selected: None,
                })
                .into_any_element()
            },
        );
        let state = list.0.borrow();
        assert_eq!(state.base_handle.bounds().size.width, px(width));
        assert_eq!(
            state.base_handle.max_offset().y,
            height * (count - 5) as f32
        );
    }
}

#[gpui::test]
fn expanding_archive_reveals_newest_once_after_rows_change(cx: &mut gpui::TestAppContext) {
    use gpui::{point, px, size};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::expanding_archive_reveals_newest_once_after_rows_change"
        ),
        cx,
        |cx, app, _, project| {
            let archived = |index| {
                let mut draft = crate::sessions::DraftSession::with_id(
                    Some(crate::agents::Backend::Pi),
                    format!("archive-{index}"),
                    project.to_path_buf(),
                );
                draft.app_session_id = index;
                draft.created_ms = index as u64;
                draft.submitted = true;
                assert!(draft.set_archived(true));
                draft
            };
            let rail = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.drafts = (1..=12).map(archived).collect();
                    let mut next = archived(13);
                    next.set_archived(false);
                    app.sessions.drafts.push(next);
                    app.sessions.archived_expanded = false;
                    app.toggle_archive(cx);
                    app.views.archived_session_rail.clone()
                })
            });
            let height = session_row_height(false);
            for newest in [12, 13] {
                cx.draw(
                    point(px(0.), px(0.)),
                    size(px(280.), height * 2.),
                    |_, _| rail.clone().into_any_element(),
                );
                cx.update(|_, cx| {
                    let view = rail.read(cx);
                    assert_eq!(view.rows[0].identity, format!("draft:archive-{newest}"));
                    assert_eq!(view.list.0.borrow().base_handle.offset().y, px(0.));
                    assert!(view.reveal.is_none());
                    assert!(view.list.0.borrow().deferred_scroll_to_item.is_none());
                });

                let offset = -(height * 3. + px(5.));
                cx.update(|_, cx| {
                    rail.update(cx, |view, cx| {
                        view.list
                            .0
                            .borrow()
                            .base_handle
                            .set_offset(point(px(0.), offset));
                        cx.notify();
                    });
                });
                cx.draw(
                    point(px(0.), px(0.)),
                    size(px(280.), height * 2.),
                    |_, _| rail.clone().into_any_element(),
                );
                cx.update(|window, cx| {
                    assert_eq!(rail.read(cx).list.0.borrow().base_handle.offset().y, offset);
                    if newest == 12 {
                        app.update(cx, |app, cx| {
                            app.toggle_archive(cx);
                            assert!(rail.read(cx).reveal.is_none());
                            app.request_draft_archive("archive-13".into(), true, window, cx);
                            app.toggle_archive(cx);
                        });
                    }
                });
            }
        },
    );
}

#[test]
fn empty_archive_consumes_top_reveal_without_scheduling_a_scroll() {
    let list = UniformListScrollHandle::new();
    let mut reveal = Some(SessionReveal::Index(0));
    reveal_archived_session_row(&list, 0, |_| None, &mut reveal);
    assert!(reveal.is_none());
    assert!(list.0.borrow().deferred_scroll_to_item.is_none());
}

#[gpui::test]
fn single_archive_change_splices_the_warm_archive(cx: &mut gpui::TestAppContext) {
    use crate::runtime::{RuntimeCommand, RuntimeEvent};
    use crate::sessions::SessionSummary;
    use gpui::{point, px, size};

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::single_archive_change_splices_the_warm_archive"
        ),
        cx,
        |cx, app, runtime, project| {
            let sessions: Vec<_> = (1..=256)
                .map(|id| {
                    SessionSummary::from_cached(
                        id.to_string(),
                        project.join(id.to_string()),
                        project.into(),
                        id.to_string(),
                        String::new(),
                        String::new(),
                        None,
                        std::time::UNIX_EPOCH,
                        1,
                        Default::default(),
                        id % 2 == 0,
                        false,
                        String::new(),
                    )
                    .with_app_session_id(id)
                })
                .collect();
            let selected = sessions[130].clone();
            let previous = sessions[131].catalog_key();
            let next = sessions[129].catalog_key();
            let rail = cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.selected_draft = None;
                    app.sessions.drafts.clear();
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    snapshot.selected_session = Some(selected.path.clone());
                    snapshot.harness = Some(selected.harness);
                    app.sessions.visible = sessions.clone().into();
                    app.sessions.all = sessions.into();
                    app.sessions.archived_expanded = true;
                    app.notify_session_rail(cx);
                    app.views.archived_session_rail.clone()
                })
            });
            let height = session_row_height(false);
            let draw = |cx: &mut gpui::VisualTestContext| {
                cx.draw(
                    point(px(0.), px(0.)),
                    size(px(280.), height * 4.),
                    |_, _| rail.clone().into_any_element(),
                )
            };
            draw(cx);
            let preparations = cx.update(|_, cx| {
                let view = rail.read(cx);
                assert_eq!(view.rows.len(), 128);
                view.list
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.), -(height * 90. + px(5.))));
                view.preparations
            });
            // Insert and remove a row above the viewport, then reject a second archive.
            for (archive, reject) in [(true, false), (false, false), (true, true)] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.request_rail_archive(selected.catalog_key(), archive, window, cx);
                    })
                });
                let check = |cx: &mut gpui::VisualTestContext, archived: bool| {
                    draw(cx);
                    cx.update(|_, cx| {
                        use crate::app::views::session_rail::ArchivedItem;
                        let view = rail.read(cx);
                        let rows = &view.rows;
                        assert_eq!(rows.len(), 128 + usize::from(archived));
                        assert!(matches!(&rows[62].item, ArchivedItem::Session(k) if *k == previous));
                        if archived {
                            assert!(matches!(&rows[63].item, ArchivedItem::Session(k) if *k == selected.catalog_key()));
                        }
                        assert!(matches!(&rows[63 + usize::from(archived)].item, ArchivedItem::Session(k) if *k == next));
                        assert_eq!(view.preparations, preparations);
                        let top = if archived { 91. } else { 90. };
                        assert_eq!(view.list.0.borrow().base_handle.offset().y, -(height * top + px(5.)));
                        let app = app.read(cx);
                        assert!(app.sessions.archived_expanded);
                        assert_eq!(app.selected_rail_root().unwrap().catalog_key(), selected.catalog_key());
                    });
                };
                check(cx, archive);
                let request = std::iter::from_fn(|| runtime.try_recv_command())
                    .find_map(|command| match command {
                        RuntimeCommand::SetSessionArchived { request, .. } => Some(request),
                        _ => None,
                    })
                    .expect("persist the clicked row");
                let mut stored = selected.clone();
                stored.archived = archive;
                runtime.send_event(RuntimeEvent::SessionArchiveResult {
                    request,
                    rows: if reject {
                        Err("fixture archive failure".into())
                    } else {
                        Ok(vec![stored])
                    },
                });
                cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
                check(cx, archive && !reject);
            }
        },
    );
}

#[gpui::test]
fn first_submit_keeps_one_row_when_catalog_arrives_first(cx: &mut gpui::TestAppContext) {
    check_first_submit_rows(
        concat!(
            module_path!(),
            "::first_submit_keeps_one_row_when_catalog_arrives_first"
        ),
        cx,
        true,
    );
}

#[gpui::test]
fn first_submit_keeps_one_row_when_path_arrives_first(cx: &mut gpui::TestAppContext) {
    check_first_submit_rows(
        concat!(
            module_path!(),
            "::first_submit_keeps_one_row_when_path_arrives_first"
        ),
        cx,
        false,
    );
}

fn check_first_submit_rows(test_name: &str, cx: &mut gpui::TestAppContext, catalog_first: bool) {
    use crate::{
        agents::Backend,
        runtime::{RuntimeCommand, RuntimeEvent, RuntimeSnapshot},
        sessions::{draft_target, normalize_session_path},
        storage::{ObservedSession, PromptSubmission, SessionFacts, SessionRepository as _},
    };

    crate::app::test_support::with_offline_app(test_name, cx, |cx, app, runtime, project| {
        crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
            .expect("trust test project");
        let id = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.sessions.preferred_harness = Some(Backend::Codex);
                app.new_session(project.into(), window, cx);
                app.sessions.selected_draft.clone().expect("new draft")
            })
        });
        let target = draft_target(&id);
        let assert_one_row = |cx: &mut gpui::VisualTestContext, expected: &str| {
            cx.draw(
                gpui::point(gpui::px(0.), gpui::px(0.)),
                gpui::size(gpui::px(280.), gpui::px(800.)),
                |_, cx| app.read(cx).views.session_rail.clone().into_any_element(),
            );
            cx.update(|_, cx| {
                let rail = app.read(cx).views.session_rail.read(cx);
                let rows = rail.rows.borrow();
                let chats: Vec<_> = rows
                    .iter()
                    .filter(|row| row.starts_with("draft:") || row.starts_with("session:"))
                    .map(String::as_str)
                    .collect();
                assert_eq!(chats, [expected]);
            });
        };
        assert_one_row(cx, &target);
        cx.run_until_parked();
        let request = std::iter::from_fn(|| runtime.try_recv_command())
            .find_map(|command| match command {
                RuntimeCommand::NewSession {
                    id: new, request, ..
                } if new == id => request,
                _ => None,
            })
            .expect("New routed to this draft");
        runtime.send_event(RuntimeEvent::SelectionRouted(request));
        runtime.send_event(RuntimeEvent::Snapshot {
            generation: 1,
            snapshot: std::sync::Arc::new(RuntimeSnapshot {
                harness: Some(Backend::Codex),
                project: project.into(),
                ..Default::default()
            }),
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.drain_runtime(cx);
                app.submit(
                    "first prompt".into(),
                    crate::protocol::PromptMode::Normal,
                    window,
                    cx,
                );
            })
        });
        let (submission, title) = std::iter::from_fn(|| runtime.try_recv_command())
            .find_map(|command| match command {
                RuntimeCommand::Prompt {
                    target: to,
                    submission_id,
                    message,
                    provisional_title,
                    ..
                } if to == target && message == "first prompt" => {
                    Some((submission_id, provisional_title))
                }
                _ => None,
            })
            .expect("first prompt sent to the new draft");
        assert_one_row(cx, &target);

        let path = normalize_session_path(
            &project
                .join("session-locators")
                .join(Backend::Codex.as_str())
                .join("native-first-prompt"),
        );
        let metadata = serde_json::from_value(serde_json::json!({
            "harness": Backend::Codex, "id": "native-first-prompt", "path": path,
            "project": project, "first_user_message": "first prompt", "is_running": true
        }))
        .expect("native session metadata");
        assert_eq!(
            cx.update(|_, cx| app.read(cx).sessions.draft_session_ids[&id]),
            0
        );
        assert_eq!(title.as_deref(), Some("first prompt"));
        let store = crate::app::persistence::shared().expect("store");
        let receipt = store
            .submit_prompt(PromptSubmission {
                target: &target,
                harness: Backend::Codex,
                project,
                session: None,
                mode: crate::protocol::PromptMode::Normal,
                message: "first prompt",
                profile_id: None,
                provisional_title: title.as_deref(),
                display_message: None,
                invocation: None,
                images: &[],
            })
            .expect("commit first prompt");
        if catalog_first {
            // A full refresh can read the caller registration before the actor
            // reports LoadState and before the UI receives its outbox receipt.
            let registry = crate::agents::CallerRegistry::default();
            let repository = store.clone();
            registry.set_execution_sinks(
                Some(std::sync::Arc::new(move |caller, family| {
                    repository.register_caller_session(caller, family)
                })),
                None,
            );
            let caller = registry.issue_main_session(
                &Default::default(),
                &crate::agents::SessionLaunch {
                    harness: Backend::Codex,
                    project: project.to_owned(),
                    start: crate::agents::SessionStart::New(Some(id.clone())),
                    session_id: None,
                    wake: None,
                    service_tier: None,
                },
            );
            caller.bind_with_locator("native-first-prompt", Some(path.clone()));
            let rows = store
                .with(|db| db.cached_sessions(""))
                .expect("read catalog during launch");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].app_session_id, receipt.app_session_id.get());
            runtime.send_event(RuntimeEvent::Sessions {
                generation: 1,
                sessions: rows.clone(),
                all_sessions: rows,
                activities: None,
            });
            cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
            assert_one_row(cx, &target);
            cx.update(|_, cx| {
                let draft = app
                    .read(cx)
                    .sessions
                    .drafts
                    .iter()
                    .find(|draft| draft.id == id)
                    .unwrap();
                assert_eq!(draft.title.as_deref(), Some("first prompt"));
            });
        }
        store
            .record_session_facts(
                ObservedSession {
                    harness: Backend::Codex,
                    project,
                    path: &path,
                    profile_id: None,
                    draft: Some(&id),
                },
                &SessionFacts::default(),
            )
            .expect("bind runtime locator");
        let row = store
            .record_session_metadata(&metadata)
            .expect("catalog row");
        assert_eq!(row.app_session_id, receipt.app_session_id.get());
        let identity = || RuntimeEvent::DraftPromptCommitted {
            provisional_title: title.clone(),
            draft: id.clone(),
            app_session_id: receipt.app_session_id,
        };
        if !catalog_first {
            runtime.send_event(identity());
        }
        let saved_key = crate::app::views::session_rail::session_row_identity(&row);
        let catalog = RuntimeEvent::SessionUpdated(row);
        let accepted = RuntimeEvent::PromptResult {
            submission_id: Some(submission),
            target: target.clone(),
            outcome: crate::agents::PromptOutcome::Accepted,
            session: Some(path.clone()),
        };
        let (first, second) = if catalog_first {
            (catalog, accepted)
        } else {
            (accepted, catalog)
        };
        runtime.send_event(first);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.drain_runtime(cx);
                let draft = app
                    .sessions
                    .drafts
                    .iter()
                    .find(|draft| draft.id == id)
                    .expect("draft before promotion");
                assert!(draft.submitted);
                assert_eq!(
                    app.sessions.submitted_drafts[&id].as_deref(),
                    draft.session_path.as_deref()
                );
                if catalog_first {
                    assert!(draft.session_path.is_none());
                    assert!(app.sessions.visible.by_path(&path).is_some());
                } else {
                    assert_eq!(draft.session_path.as_ref(), Some(&path));
                    assert!(app.sessions.visible.by_path(&path).is_none());
                }
            })
        });
        assert_one_row(cx, &target);
        if catalog_first {
            runtime.send_event(identity());
        }
        runtime.send_event(second);
        cx.update(|_, cx| app.update(cx, |app, cx| app.drain_runtime(cx)));
        assert_one_row(cx, &saved_key);
    });
}

#[gpui::test]
fn switching_grouping_remeasures_session_rows(cx: &mut gpui::TestAppContext) {
    use gpui::{ParentElement as _, Styled as _, point, px, size};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::switching_grouping_remeasures_session_rows"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let mut draft = crate::sessions::DraftSession::with_id(
                        Some(crate::agents::Backend::Pi),
                        "layout-test".into(),
                        project.to_path_buf(),
                    );
                    draft.app_session_id = 1;
                    app.sessions.drafts = vec![draft];
                    app.notify_session_rail(cx);
                })
            });
            for grouped in [false, true, false] {
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        if app.settings.group_sessions_by_project != grouped {
                            app.toggle_settings_project_groups(cx);
                        }
                    })
                });
                cx.draw(
                    point(px(0.0), px(0.0)),
                    size(px(280.0), px(800.0)),
                    |_, cx| {
                        gpui::div()
                            .w(px(280.0))
                            .h(px(800.0))
                            .child(app.read(cx).views.session_rail.clone())
                    },
                );
                cx.update(|_, cx| {
                    let rail = app.read(cx).views.session_rail.read(cx);
                    let bounds = rail
                        .list
                        .bounds_for_item(usize::from(grouped))
                        .expect("session bounds");
                    assert_eq!(bounds.size.height, session_row_height(grouped));
                    assert_eq!(
                        rail.rows.borrow().iter().any(|row| row == "new-folder"),
                        !grouped
                    );
                });
            }
        },
    );
}

#[test]
fn revealing_profile_copies_uses_the_requested_active_or_archived_row() {
    use crate::app::views::session_rail::session_row_identity;
    use crate::sessions::{SessionSummary, UsageSummary};

    for archived in [false, true] {
        let sessions = [41, 84].map(|id| {
            let mut session = SessionSummary::from_cached(
                "same-native".into(),
                format!("/profiles/{id}/session").into(),
                "/project".into(),
                String::new(),
                String::new(),
                String::new(),
                None,
                std::time::SystemTime::UNIX_EPOCH,
                0,
                UsageSummary::default(),
                archived,
                false,
                String::new(),
            )
            .with_app_session_id(id);
            session.profile_id = Some(id.to_string());
            session
        });
        let keys = sessions.each_ref().map(session_row_identity);
        let rows = RefCell::new(keys.iter().cloned().collect());
        let list = ListState::new(2, ListAlignment::Top, gpui::px(0.0))
            .with_uniform_item_height(session_row_height(false));
        let archived_list = UniformListScrollHandle::new();
        for index in [1, 0, 1] {
            let mut reveal = Some(SessionReveal::SessionID(session_row_identity(
                &sessions[index],
            )));
            if archived {
                reveal_archived_session_row(
                    &archived_list,
                    keys.len(),
                    |id| keys.iter().position(|row| row == id),
                    &mut reveal,
                );
                assert_eq!(
                    archived_list
                        .0
                        .borrow()
                        .deferred_scroll_to_item
                        .unwrap()
                        .item_index,
                    index
                );
            } else {
                reveal_session_row(&list, &rows, &mut reveal);
                assert_eq!(list.logical_scroll_top().item_ix, index);
            }
            assert!(reveal.is_none());
        }
    }
}
