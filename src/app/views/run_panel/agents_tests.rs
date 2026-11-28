use super::*;

#[gpui::test]
fn conversation_selection_and_focus_keep_the_text_anchor(cx: &mut gpui::TestAppContext) {
    struct Row {
        focus: gpui::FocusHandle,
        selected: bool,
        width: f32,
    }
    impl gpui::Render for Row {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
                .w(gpui::px(self.width))
                .px(crate::app::ui::grid::inspector_inset())
                .child(
                    conversation_row("conversation", self.selected)
                        .track_focus(&self.focus)
                        .child(
                            div()
                                .debug_selector(|| "conversation-text".into())
                                .child("Main agent"),
                        ),
                )
        }
    }
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, cx| Row {
        focus: cx.focus_handle(),
        selected: false,
        width: 332.0,
    });
    for width in [220.0, 332.0, 450.0] {
        cx.update(|window, cx| {
            view.update(cx, |row, _| {
                row.width = width;
                row.selected = false;
            });
            window.blur(cx);
            window.draw(cx).clear(cx);
        });
        let before = cx.debug_bounds("conversation-text").unwrap();
        assert_eq!(before.left(), crate::app::ui::grid::inspector_inset());
        cx.update(|window, cx| {
            view.update(cx, |row, _| row.selected = true);
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
            window.draw(cx).clear(cx);
        });
        assert_eq!(cx.debug_bounds("conversation-text").unwrap(), before);
    }
}
