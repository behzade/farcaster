use super::*;
use crate::{app::session_folders::SessionFolder, projects::DraftSession};
use gpui::{AppContext as _, StatefulInteractiveElement as _};

fn draft(id: i64) -> ActiveSessionItem {
    let mut draft = DraftSession::with_id("pi".into(), format!("draft-{id}"), "/project".into());
    draft.app_session_id = id;
    ActiveSessionItem::Draft(draft)
}

#[test]
fn folder_headers_follow_unfiled_sessions_and_keep_empty_folders() {
    let mut folders = SessionFolders::default();
    folders.folders = vec![
        SessionFolder {
            id: 1,
            name: "Work".into(),
        },
        SessionFolder {
            id: 2,
            name: "Empty".into(),
        },
    ];
    folders.assign(3, Some(1));
    let rows = folder_rows(vec![draft(3), draft(2), draft(1)], &folders);
    assert!(matches!(&rows[0], FolderRow::Session(item) if item.app_session_id() == 2));
    assert!(matches!(&rows[1], FolderRow::Session(item) if item.app_session_id() == 1));
    assert!(matches!(&rows[2], FolderRow::Header(1, _)));
    assert!(matches!(&rows[3], FolderRow::Session(item) if item.app_session_id() == 3));
    assert!(matches!(&rows[4], FolderRow::Header(2, _)));
    assert!(matches!(&rows[5], FolderRow::New));
}

#[test]
fn folder_membership_survives_draft_submission_filtering_and_archive_restore() {
    use super::super::groups::session_rail_lists;
    use crate::sessions::{SessionSummary, UsageSummary};
    let mut folders = SessionFolders::default();
    folders.folders.push(SessionFolder {
        id: 1,
        name: "Work".into(),
    });
    folders.assign(7, Some(1));
    let session = SessionSummary::from_cached(
        "persisted".into(),
        "/sessions/7.jsonl".into(),
        "/project".into(),
        "Session".into(),
        String::new(),
        String::new(),
        None,
        std::time::SystemTime::UNIX_EPOCH,
        0,
        UsageSummary::default(),
        false,
        false,
        String::new(),
    )
    .with_app_session_id(7);
    let ActiveSessionItem::Draft(mut draft) = draft(7) else {
        unreachable!()
    };
    draft.submitted = true;
    let promoted = session_rail_lists(std::slice::from_ref(&session), &[draft], None, &[]);
    let rows = folder_rows(promoted.active, &folders);
    assert!(matches!(&rows[0], FolderRow::Header(1, _)));
    assert!(matches!(&rows[1], FolderRow::Session(item) if item.app_session_id() == 7));
    assert_eq!(rows.len(), 3);

    let filtered = session_rail_lists(
        std::slice::from_ref(&session),
        &[],
        Some(std::path::Path::new("/other")),
        &[],
    );
    assert!(filtered.active.is_empty());
    assert_eq!(folders.folder_for(7), Some(1));

    let mut archived = session.clone();
    archived.archived = true;
    let lists = session_rail_lists(&[archived], &[], None, &[]);
    assert_eq!(lists.archived.len(), 1);
    assert_eq!(folder_rows(lists.active, &folders).len(), 2);
    let restored = session_rail_lists(&[session], &[], None, &[]);
    assert!(
        matches!(&folder_rows(restored.active, &folders)[1], FolderRow::Session(item) if item.app_session_id() == 7)
    );
}

struct FolderDropHarness {
    creation_button: bool,
    received: std::rc::Rc<std::cell::Cell<Option<i64>>>,
}

impl gpui::Render for FolderDropHarness {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let received = self.received.clone();
        let creation_button = self.creation_button;
        let drag = super::DraggedSession {
            app_session_id: 7,
            path: Some("/session".into()),
            kind: super::SessionRailKind::Project,
            title: "Session".into(),
            project: "Project".into(),
        };
        // Match the sidebar's virtual list and its enclosing drop area.
        div().w(gpui::px(300.)).h(gpui::px(200.)).child(
            div()
                .id("outer-drop-area")
                .size_full()
                .can_drop(|_, _, _| false)
                .on_drop(|_: &super::DraggedSession, _, _| panic!("outer drop stole the session"))
                .child(
                    gpui::list(
                        gpui::ListState::new(2, gpui::ListAlignment::Top, gpui::px(0.)),
                        move |index, _, _| {
                            if index == 0 {
                                div()
                                    .id("source")
                                    .w_full()
                                    .h(gpui::px(40.))
                                    .on_drag(drag.clone(), |drag, _, _, cx| {
                                        cx.new(|_| drag.clone())
                                    })
                                    .child("Session")
                                    .into_any_element()
                            } else {
                                let received = received.clone();
                                let content = if creation_button {
                                    new_folder_button(|_, _| {}).into_any_element()
                                } else {
                                    div().child("Work").into_any_element()
                                };
                                super::folder_drop_target(
                                    div().id("folder").h(gpui::px(40.)).child(content),
                                    move |drag, _, _| received.set(Some(drag.app_session_id)),
                                )
                                .into_any_element()
                            }
                        },
                    )
                    .size_full(),
                ),
        )
    }
}

#[gpui::test]
fn folder_drop_accepts_session_across_header_width(cx: &mut gpui::TestAppContext) {
    check_folder_drop(cx, false);
}

#[gpui::test]
fn folder_creation_button_accepts_session_drop(cx: &mut gpui::TestAppContext) {
    check_folder_drop(cx, true);
}

fn check_folder_drop(cx: &mut gpui::TestAppContext, creation_button: bool) {
    use gpui::{MouseButton, point, px};
    cx.update(gpui_component::init);
    let received = std::rc::Rc::new(std::cell::Cell::new(None));
    let (_, cx) = cx.add_window_view(|_, _| FolderDropHarness {
        received: received.clone(),
        creation_button,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for x in [15., 150., 285.] {
        received.set(None);
        cx.simulate_mouse_down(
            point(px(15.), px(15.)),
            MouseButton::Left,
            Default::default(),
        );
        cx.simulate_mouse_move(
            point(px(35.), px(15.)),
            Some(MouseButton::Left),
            Default::default(),
        );
        cx.simulate_mouse_move(
            point(px(x), px(60.)),
            Some(MouseButton::Left),
            Default::default(),
        );
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_mouse_up(point(px(x), px(60.)), MouseButton::Left, Default::default());
        assert_eq!(received.get(), Some(7), "drop at x={x}");
    }
}
