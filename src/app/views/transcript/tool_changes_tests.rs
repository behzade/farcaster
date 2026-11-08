use super::file_label;
use std::path::Path;

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
