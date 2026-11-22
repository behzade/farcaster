use super::*;
use crate::repository::RepositoryBackend;
use std::path::Path;

fn git_backend(project: &Path) -> RepositoryBackend {
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(project)
            .status()
            .unwrap()
            .success()
    );
    RepositoryBackend::discover(project, BackendPreference::Git)
        .unwrap()
        .unwrap()
}

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
fn file_counts_and_labels_stay_aligned_during_hover_focus_and_selection(
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
                                .repository_change_row(
                                    &self.change,
                                    crate::app::ui::primitives::DiffColumns::for_counts(
                                        self.change.counts,
                                    ),
                                    self.app.clone(),
                                )
                                .unwrap(),
                        ),
                )
        }
    }
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::file_counts_and_labels_stay_aligned_during_hover_focus_and_selection"
        ),
        cx,
        |cx, app, _, project| {
            let backend = git_backend(project);
            std::fs::write(project.join("long_file_name.rs"), "fn main() {}\n").unwrap();
            let snapshot = backend.snapshot().unwrap();
            let change = snapshot.changes[0].clone();
            let key = change.target.key.clone();
            let row_id = repository_row_id(&key);

            let counts_selector = Box::leak(format!("repository-counts-{row_id}").into_boxed_str());
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
                    let before = [counts_selector as &str, filename_selector as &str]
                        .map(|s| cx.debug_bounds(s).unwrap());
                    assert_eq!(before[0].right(), px(width));
                    assert_eq!(
                        before[0].right(),
                        cx.debug_bounds("folder-summary").unwrap().right()
                    );
                    assert_eq!(before[1].left(), crate::app::ui::grid::tree_indent(depth));
                    assert!(before[1].size.width > px(0.0));
                    cx.simulate_mouse_move(before[1].center(), None, Default::default());
                    cx.update(|window, cx| {
                        focus.focus(window, cx);
                        window.draw(cx).clear(cx);
                    });
                    assert_eq!(
                        [counts_selector as &str, filename_selector as &str]
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
                        [counts_selector as &str, filename_selector as &str]
                            .map(|s| cx.debug_bounds(s).unwrap()),
                        before
                    );
                }
            }
        },
    );
}

#[gpui::test]
fn long_change_list_scrolls_below_the_filter(cx: &mut gpui::TestAppContext) {
    use gpui::{ScrollDelta, ScrollEvent, ScrollSource, px};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::long_change_list_scrolls_below_the_filter"
        ),
        cx,
        |cx, app, _, project| {
            let project = project.canonicalize().unwrap();
            let backend = git_backend(&project);
            for i in 0..60 {
                std::fs::write(project.join(format!("changed-{i:02}.rs")), "").unwrap();
            }
            let snapshot = backend.snapshot().unwrap();
            let last = snapshot
                .changes
                .iter()
                .find(|change| change.relative_path.ends_with("changed-59.rs"))
                .unwrap();
            let selector: &'static str = Box::leak(
                format!(
                    "repository-filename-{}",
                    repository_row_id(&last.target.key)
                )
                .into_boxed_str(),
            );
            cx.simulate_resize(gpui::size(px(1600.), px(660.)));
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    let repo = &mut app.project.repository;
                    repo.project = project.clone();
                    repo.execution_allowed = true;
                    for change in &snapshot.changes {
                        repo.row_focus
                            .insert(change.target.key.clone(), cx.focus_handle());
                    }
                    repo.backend = Some(backend);
                    repo.snapshot = Some(snapshot);
                    app.views.run_panel.update(cx, |panel, cx| {
                        panel.changes.set_all(&project, true);
                        cx.notify();
                    });
                    cx.notify();
                })
            });
            let draw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| window.draw(cx).clear(cx))
            };
            draw(cx);
            let header = cx.debug_bounds("repository-header-and-filter").unwrap();
            let list = cx.debug_bounds("repository-files").unwrap();
            assert!(list.top() >= header.bottom());
            assert!(
                list.bottom() <= px(660.),
                "file list extends below the window: {list:?}"
            );
            let initial = cx.debug_bounds(selector);
            assert!(initial.is_none_or(|last| last.top() >= list.bottom()));
            let wheel = |cx: &mut gpui::VisualTestContext, dy| {
                cx.simulate_event(ScrollEvent {
                    source: ScrollSource::Pointer,
                    position: list.center(),
                    delta: ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
                    ..Default::default()
                });
                draw(cx);
            };
            wheel(cx, -10000.);
            let last = cx
                .debug_bounds(selector)
                .expect("the final file should be reachable");
            assert!(
                last.top() >= list.top() && last.bottom() <= list.bottom(),
                "last={last:?} initial={initial:?} list={list:?}"
            );
            assert_eq!(
                cx.debug_bounds("repository-header-and-filter"),
                Some(header)
            );
            wheel(cx, 10000.);
            assert_eq!(cx.debug_bounds(selector), initial);
        },
    );
}
