use super::*;

#[test]
fn first_archive_expansion_does_not_scroll_past_the_selected_row() {
    let list = ListState::new(12, ListAlignment::Top, gpui::px(0.0))
        .with_uniform_item_height(session_row_height(false));
    let rows = RefCell::new((0..12).map(|index| format!("session:{index}")).collect());
    let mut reveal = Some("session:5".to_owned());
    reveal_session_row(&list, &rows, &mut reveal);
    assert_eq!(list.logical_scroll_top().item_ix, 5);
    assert_eq!(list.logical_scroll_top().offset_in_item, gpui::px(0.0));
    assert!(reveal.is_none());
    reveal = Some("session:4".to_owned());
    reveal_session_row(&list, &rows, &mut reveal);
    assert_eq!(list.logical_scroll_top().item_ix, 4);
    assert_eq!(list.logical_scroll_top().offset_in_item, gpui::px(0.0));
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
            .with_uniform_item_height(session_row_height(archived));
        for index in [1, 0, 1] {
            let mut reveal = Some(session_row_identity(&sessions[index]));
            reveal_session_row(&list, &rows, &mut reveal);
            assert_eq!(list.logical_scroll_top().item_ix, index);
            assert!(reveal.is_none());
        }
    }
}
