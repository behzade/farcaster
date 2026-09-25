use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, Styled as _, Window, div, point, px, size,
};
use gpui_component::input::{Textarea, TextareaState};

struct DraftView {
    root_focus: FocusHandle,
    composer: Entity<TextareaState>,
    composer_height: Pixels,
}

impl Render for DraftView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.composer.read(cx).focus_handle(cx);
        div()
            .debug_selector(|| "draft-viewport".into())
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.root_focus)
            .child(super::render_body(
                div()
                    .debug_selector(|| "draft-composer".into())
                    .h(self.composer_height)
                    .flex_none()
                    .child(Textarea::new(&self.composer)),
                None,
                focus,
            ))
    }
}

#[gpui::test]
fn draft_composer_centers_on_resize_without_clipping_tall_content(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| DraftView {
        root_focus: cx.focus_handle(),
        composer: cx.new(|cx| TextareaState::new(window, cx)),
        composer_height: px(100.0),
    });
    for height in [360.0, 600.0, 1000.0] {
        cx.simulate_resize(size(px(800.0), px(height)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let viewport = cx
            .debug_bounds("draft-viewport")
            .expect("viewport rendered");
        let composer = cx
            .debug_bounds("draft-composer")
            .expect("composer rendered");
        let offset = f32::from(composer.center().y - viewport.center().y).abs();
        assert!(
            offset < 1.0,
            "composer is not centered at height {height}: {offset}"
        );
    }

    cx.simulate_resize(size(px(800.0), px(360.0)));
    view.update(cx, |view, cx| {
        view.composer_height = px(800.0);
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let viewport = cx
        .debug_bounds("draft-viewport")
        .expect("viewport rendered");
    let composer = cx
        .debug_bounds("draft-composer")
        .expect("composer rendered");
    assert!(
        composer.top() >= viewport.top(),
        "tall content must start inside the viewport"
    );
    assert_eq!(composer.size.height, px(800.0));
    assert!(
        composer.bottom() > viewport.bottom(),
        "tall content must overflow rather than shrink"
    );
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
            composer_height: px(100.0),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // Blank space above the composer still focuses the input.
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
