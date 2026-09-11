use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Styled as _, Window, div, point, px,
};
use gpui_component::input::{Textarea, TextareaState};

struct DraftView {
    root_focus: FocusHandle,
    composer: Entity<TextareaState>,
}

impl Render for DraftView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.composer.read(cx).focus_handle(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.root_focus)
            .child(super::render_body(
                div().h(px(100.0)).child(Textarea::new(&self.composer)),
                None,
                focus,
                px(600.0),
            ))
    }
}

#[gpui::test]
fn draft_background_keeps_composer_focus(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let composer = cx.new(|cx| TextareaState::new(window, cx));
        composer.read(cx).focus_handle(cx).focus(window, cx);
        DraftView {
            root_focus: cx.focus_handle(),
            composer,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // Above the composer, inside the draft body's blank top padding.
    cx.simulate_click(point(px(40.0), px(20.0)), Default::default());
    cx.update(|window, cx| {
        assert!(
            view.read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    cx.simulate_input("Draft text");
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).composer.read(cx).value().as_ref(),
            "Draft text"
        );
    });
}
