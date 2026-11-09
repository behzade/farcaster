use super::*;

#[test]
fn auto_does_not_select_jj_before_discovery() {
    assert_eq!(
        selected_backend(None, BackendPreference::Auto, true, true),
        None
    );
    assert_eq!(
        selected_backend(None, BackendPreference::Jujutsu, true, false),
        None
    );
    assert_eq!(
        selected_backend(
            Some(RepositoryKind::Git),
            BackendPreference::Auto,
            true,
            true,
        ),
        Some(RepositoryKind::Git)
    );
}

#[gpui::test]
fn file_status_and_labels_stay_aligned_during_hover_focus_and_selection(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{ParentElement as _, Render, Styled as _, px};
    struct Row {
        app: gpui::WeakEntity<FarcasterApp>,
        change: WorkingCopyChange,
        width: f32,
        depth: usize,
    }
    impl Render for Row {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let app = self.app.upgrade().unwrap();
            div()
                .w(px(self.width))
                .flex()
                .flex_col()
                .child(
                    crate::app::ui::primitives::tree_folder_row(
                        "folder",
                        "src/app".into(),
                        self.depth,
                        false,
                        true,
                        |_, _| {},
                    )
                    .child(
                        crate::app::ui::primitives::folder_change_summary(
                            1,
                            Some((1, 0)),
                            crate::app::ui::primitives::DiffColumns::for_counts([(1, 0)]),
                        )
                        .debug_selector(|| "folder-summary".into()),
                    ),
                )
                .child(
                    div()
                        .pl(crate::app::ui::grid::tree_indent(self.depth))
                        .child(
                            app.read(cx)
                                .repository_change_row(&self.change, self.app.clone())
                                .unwrap(),
                        ),
                )
        }
    }
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::file_status_and_labels_stay_aligned_during_hover_focus_and_selection"
        ),
        cx,
        |cx, app, _, project| {
            assert!(
                std::process::Command::new("git")
                    .args(["init", "-q"])
                    .current_dir(project)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::write(project.join("long_file_name.rs"), "fn main() {}\n").unwrap();
            let snapshot =
                crate::repository::RepositoryBackend::discover(project, BackendPreference::Git)
                    .unwrap()
                    .unwrap()
                    .snapshot()
                    .unwrap();
            let change = snapshot.changes[0].clone();
            let key = change.target.key.clone();
            let row_id = repository_row_id(&key);
            let status_selector = Box::leak(format!("repository-status-{row_id}").into_boxed_str());
            let filename_selector =
                Box::leak(format!("repository-filename-{row_id}").into_boxed_str());
            let focus = cx.update(|_, cx| {
                let focus = cx.focus_handle();
                app.update(cx, |app, _| {
                    app.project.repository.execution_allowed = true;
                    app.project.repository.row_focus.insert(key, focus.clone());
                });
                focus
            });
            for width in [220.0, 332.0, 450.0] {
                for depth in [0, 2] {
                    cx.update(|window, cx| {
                        app.update(cx, |app, _| {
                            app.project.repository.edits.selection.paths.clear()
                        });
                        window.replace_root(cx, |_, _| Row {
                            app: app.downgrade(),
                            change: change.clone(),
                            width,
                            depth,
                        });
                        window.draw(cx).clear(cx);
                    });
                    let before = [status_selector as &str, filename_selector as &str]
                        .map(|s| cx.debug_bounds(s).unwrap());
                    assert_eq!(before[0].right(), px(width));
                    assert_eq!(
                        before[0].right(),
                        cx.debug_bounds("folder-summary").unwrap().right()
                    );
                    assert_eq!(
                        before[1].left(),
                        crate::app::ui::grid::tree_indent(depth)
                            + crate::app::ui::grid::detail_indent()
                    );
                    assert!(before[1].size.width > px(0.0));
                    cx.simulate_mouse_move(before[1].center(), None, Default::default());
                    cx.update(|window, cx| {
                        focus.focus(window, cx);
                        window.draw(cx).clear(cx);
                    });
                    assert_eq!(
                        [status_selector as &str, filename_selector as &str]
                            .map(|s| cx.debug_bounds(s).unwrap()),
                        before
                    );
                    cx.update(|window, cx| {
                        app.update(cx, |app, _| {
                            app.project
                                .repository
                                .edits
                                .selection
                                .paths
                                .insert(change.relative_path.clone());
                        });
                        window.draw(cx).clear(cx);
                    });
                    assert_eq!(
                        [status_selector as &str, filename_selector as &str]
                            .map(|s| cx.debug_bounds(s).unwrap()),
                        before
                    );
                }
            }
        },
    );
}
