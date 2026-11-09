use super::file_label;
use std::path::Path;

#[gpui::test]
fn folder_and_file_diff_columns_share_the_same_right_edges(cx: &mut gpui::TestAppContext) {
    use gpui::{
        ParentElement as _, Styled as _, div, point, prelude::FluentBuilder as _, px, size,
    };
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let columns = crate::app::ui::primitives::DiffColumns::for_counts([(123456, 123456)]);
    for width in [220.0, 332.0, 640.0] {
        let mut expected = None;
        for folder in [false, true] {
            for counts in [(1, 0), (120, 37), (999, 1000), (123456, 123456)] {
                for depth in [0, 2] {
                    cx.draw(
                        point(px(0.0), px(0.0)),
                        size(px(width), px(100.0)),
                        |_, _| {
                            div()
                                .w(px(width))
                                .font_family(crate::app::ui::theme::UI_FONT_FAMILY)
                                .text_size(crate::app::ui::theme::theme().type_scale.body_small)
                                .pl(crate::app::ui::grid::tree_indent(depth))
                                .child(
                                    crate::app::ui::grid::row()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .child("a long filename or folder"),
                                        )
                                        .when(folder, |row| {
                                            row.child(
                                                crate::app::ui::primitives::folder_change_summary(
                                                    25,
                                                    Some(counts),
                                                    columns,
                                                ),
                                            )
                                        })
                                        .when(!folder, |row| {
                                            row.children(super::change_counts(counts, columns))
                                        }),
                                )
                        },
                    );
                    let columns = ["diff-added-column", "diff-removed-column"]
                        .map(|selector| cx.debug_bounds(selector).unwrap());
                    let edges = columns.map(|bounds| bounds.right());
                    assert_eq!(*expected.get_or_insert(edges), edges);
                    assert_eq!(edges[1], px(width));
                    assert!(columns[0].right() <= columns[1].left());
                }
            }
        }
    }
}

#[gpui::test]
fn command_and_folder_focus_do_not_move_their_content(cx: &mut gpui::TestAppContext) {
    use gpui::{InteractiveElement as _, ParentElement as _, Render, Styled as _, div, px};
    struct Rows(gpui::FocusHandle, gpui::FocusHandle);
    impl Render for Rows {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            div()
                .w(px(300.0))
                .flex()
                .flex_col()
                .child(
                    super::title_row("command-focus", "command".into(), |_, _| {})
                        .track_focus(&self.0)
                        .child(
                            div()
                                .debug_selector(|| "command-content".into())
                                .child("cargo test"),
                        ),
                )
                .child(
                    crate::app::ui::primitives::tree_folder_row(
                        "folder-focus",
                        "src/app".into(),
                        2,
                        false,
                        true,
                        |_, _| {},
                    )
                    .track_focus(&self.1)
                    .child(
                        div()
                            .debug_selector(|| "folder-count".into())
                            .child("5 files"),
                    ),
                )
        }
    }
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, cx| Rows(cx.focus_handle(), cx.focus_handle()));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let selectors = ["command-content", "folder-count"];
    let before = selectors.map(|s| cx.debug_bounds(s).unwrap());
    for command in [true, false] {
        cx.update(|window, cx| {
            let row = view.read(cx);
            let focus = if command {
                row.0.clone()
            } else {
                row.1.clone()
            };
            focus.focus(window, cx);
            window.draw(cx).clear(cx);
        });
        assert_eq!(selectors.map(|s| cx.debug_bounds(s).unwrap()), before);
    }
}

#[test]
fn file_labels_prefer_project_then_home() {
    let project = Some(Path::new("/home/user/repo"));
    let home = Some(Path::new("/home/user"));
    for (path, expected) in [
        ("/home/user/repo/src/main.rs", "src/main.rs"),
        ("src/main.rs", "src/main.rs"),
        ("../notes.txt", "~/notes.txt"),
        ("/home/user/repo-other/file.rs", "~/repo-other/file.rs"),
        ("/home/user-other/file.rs", "/home/user-other/file.rs"),
        ("/etc/config", "/etc/config"),
    ] {
        assert_eq!(file_label(path, project, home), expected, "{path}");
    }
}

#[gpui::test]
fn disclosure_hover_does_not_cover_content_and_click_still_activates(
    cx: &mut gpui::TestAppContext,
) {
    use crate::app::ui::primitives::{init_tooltips, tooltip_overlay};
    use gpui::{
        AppContext as _, InteractiveElement as _, ParentElement as _, Render, Styled as _, div, px,
    };
    use std::{cell::Cell, rc::Rc, time::Duration};

    struct Rows(Rc<Cell<usize>>);
    impl Render for Rows {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let clicks = self.0.clone();
            div()
                .w(px(300.0))
                .child(
                    super::title_row(
                        "command",
                        format!("Expand {} details", "long command ".repeat(40)),
                        move |_, _| clicks.set(clicks.get() + 1),
                    )
                    .debug_selector(|| "command-hover".into())
                    .child("cargo test"),
                )
                .child(
                    super::file_row("file", "src/main.rs".into(), true, |_, _, _| {})
                        .debug_selector(|| "file-hover".into())
                        .child("main.rs"),
                )
        }
    }
    cx.update(|cx| {
        gpui_component::init(cx);
        init_tooltips(cx);
    });
    let clicks = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let rows = cx.new(|_| Rows(clicks.clone()));
        gpui_base::Root::new(rows, window, cx)
    });
    let overlay = cx.update(|window, cx| tooltip_overlay(window, cx).unwrap());
    let command = cx.debug_bounds("command-hover").unwrap();
    let file = cx.debug_bounds("file-hover").unwrap();
    for (bounds, visible) in [(command, false), (file, true), (command, false)] {
        cx.simulate_mouse_move(bounds.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|_, cx| assert_eq!(overlay.read(cx).is_visible(), visible));
    }
    cx.simulate_click(command.center(), Default::default());
    assert_eq!(clicks.get(), 1);
}
