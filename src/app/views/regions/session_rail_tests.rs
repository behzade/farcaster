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
        let height = session_row_height(true);
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
    let rows = RefCell::new((0..12).map(|index| format!("session:{index}")).collect());
    let height = session_row_height(true);
    for selected in [5, 4] {
        let mut reveal = Some(format!("session:{selected}"));
        reveal_archived_session_row(&list, &rows, &mut reveal);
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
    let height = session_row_height(true);
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
        let rows = RefCell::new(keys.to_vec());
        let list = ListState::new(2, ListAlignment::Top, gpui::px(0.0))
            .with_uniform_item_height(session_row_height(false));
        let archived_list = UniformListScrollHandle::new();
        for index in [1, 0, 1] {
            let mut reveal = Some(session_row_identity(&sessions[index]));
            if archived {
                reveal_archived_session_row(&archived_list, &rows, &mut reveal);
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
