use super::*;
use crate::{agents::Backend, sessions::DraftSession};
use gpui::{Entity, MouseButton, point, px};

struct RailHarness(Entity<FarcasterApp>);
impl Render for RailHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .w(px(280.0))
            .h(px(800.0))
            .child(self.0.read(cx).views.session_rail.clone())
    }
}

#[gpui::test]
fn dragging_group_titles_persists_order_without_toggling_or_refiling(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::dragging_group_titles_persists_order_without_toggling_or_refiling"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, _| {
                    app.sessions.drafts = [(1, "/a"), (2, "/z")]
                        .into_iter()
                        .map(|(id, project)| {
                            let mut draft = DraftSession::with_id(
                                Some(Backend::Pi),
                                format!("draft-{id}"),
                                project.into(),
                            );
                            draft.app_session_id = id;
                            draft
                        })
                        .collect();
                    app.sessions.folders.create("First".into(), Some(1));
                    app.sessions.folders.create("Last".into(), Some(2));
                    for id in [1, 2] {
                        app.sessions.folders.set_collapsed(id, true);
                    }
                    app.sessions
                        .collapsed_projects
                        .extend([PathBuf::from("/a"), PathBuf::from("/z")]);
                    app.remember_rail_projects([PathBuf::from("/a"), PathBuf::from("/z")]);
                });
                window.replace_root(cx, |_, _| RailHarness(app.clone()));
            });
            for grouped in [false, true] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        if app.settings.group_sessions_by_project != grouped {
                            app.toggle_settings_project_groups(cx);
                        }
                    });
                    window.draw(cx).clear(cx);
                });
                let (source, target) = if grouped {
                    ("session-project-/z", "session-project-/a")
                } else {
                    ("session-folder-2", "session-folder-1")
                };
                let source = cx.debug_bounds(source).expect("source header");
                let target = cx.debug_bounds(target).expect("target header");
                let from = point(source.left() + px(18.0), source.center().y);
                let to = point(target.left() + px(18.0), target.top() + px(4.0));
                cx.simulate_mouse_down(from, MouseButton::Left, Default::default());
                cx.simulate_mouse_move(
                    point(from.x + px(15.0), from.y),
                    Some(MouseButton::Left),
                    Default::default(),
                );
                cx.simulate_mouse_move(to, Some(MouseButton::Left), Default::default());
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                });
                cx.update(|_, cx| {
                    let target = if grouped {
                        GroupTarget::Project("/a".into())
                    } else {
                        GroupTarget::Folder(1)
                    };
                    assert_eq!(
                        app.read(cx).sessions.group_drop_target,
                        Some((target, ReorderPosition::Before))
                    );
                });
                cx.simulate_mouse_up(to, MouseButton::Left, Default::default());
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                    let app = app.read(cx);
                    assert!(app.sessions.group_drop_target.is_none());
                    assert!(
                        app.sessions
                            .folders
                            .folders
                            .iter()
                            .all(|folder| folder.collapsed)
                    );
                    assert_eq!(app.sessions.collapsed_projects.len(), 2);
                    assert_eq!(app.sessions.folders.folder_for(1), Some(1));
                    assert_eq!(app.sessions.folders.folder_for(2), Some(2));
                    let saved = crate::app::persistence::open()
                        .expect("store")
                        .load_session_folders()
                        .expect("folders");
                    assert_eq!(
                        saved.folders.iter().map(|f| f.id).collect::<Vec<_>>(),
                        [2, 1]
                    );
                    let expected = if grouped { ["/z", "/a"] } else { ["/a", "/z"] };
                    assert_eq!(
                        saved.project_order,
                        std::iter::once(project.to_path_buf())
                            .chain(expected.map(PathBuf::from))
                            .collect::<Vec<_>>()
                    );
                });
            }
        },
    );
}
