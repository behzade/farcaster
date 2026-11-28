use super::*;
use gpui::{AppContext as _, Render, TestAppContext};

struct Preview(CommandDetail);

impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div().size_full().child(preview(&self.0, |_, _| {}))
    }
}

#[gpui::test]
fn short_content_fits_and_long_content_is_capped(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    for (lines, short) in [(5, true), (1000, false)] {
        let (_, cx) = cx.add_window_view(|window, cx| {
            let content = cx.new(|cx| {
                let text = "cargo test\n".repeat(lines);
                Preview(CommandDetail {
                    title: "Script",
                    markdown: format!("```\n{text}```").into(),
                    text: text.into(),
                    cwd: Some("/Users/example/project".into()),
                    focus: cx.focus_handle(),
                    return_focus: None,
                })
            });
            gpui_base::Root::new(content, window, cx)
        });
        let bounds = cx.debug_bounds("command-detail-surface").unwrap();
        let viewport = cx.update(|window, _| window.viewport_size());
        assert!(
            bounds.size.height <= viewport.height * 0.8 + gpui::px(1.0),
            "{bounds:?}"
        );
        if short {
            assert!(bounds.size.height < gpui::px(300.0), "{bounds:?}");
            assert!(bounds.size.height > gpui::px(100.0), "{bounds:?}");
        } else {
            assert!(bounds.size.height > viewport.height * 0.7, "{bounds:?}");
        }
    }
}
