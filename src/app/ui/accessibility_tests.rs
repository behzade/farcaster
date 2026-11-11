use std::{cell::Cell, rc::Rc};

use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use gpui_base::ElementExt as _;

use super::{cached_panel, defer_accessibility_draw};

struct Panel {
    prepaints: Rc<Cell<usize>>,
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let prepaints = self.prepaints.clone();
        div()
            .id("panel")
            .size_full()
            .child(
                div()
                    .id("control")
                    .role(gpui::Role::Button)
                    .aria_label("Control"),
            )
            .on_prepaint(move |_, _, _| prepaints.set(prepaints.get() + 1))
    }
}

struct Harness {
    panel: Entity<Panel>,
    accessibility_active: bool,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(cached_panel(self.panel.clone(), self.accessibility_active))
    }
}

#[gpui::test]
fn accessible_panels_prepaint_after_parent_redraws_and_cache_transitions(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::AppContext as _;

    let prepaints = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        panel: cx.new(|_| Panel {
            prepaints: prepaints.clone(),
        }),
        accessibility_active: false,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let cached = prepaints.get();
    assert!(cached > 0);
    for _ in 0..3 {
        cx.update(|window, cx| {
            view.update(cx, |_, cx| cx.notify());
            window.draw(cx).clear(cx);
        });
        assert_eq!(
            prepaints.get(),
            cached,
            "normal frames should keep the panel cache"
        );
    }

    // Activation must work even when the previous frame reused the panel's cache.
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.accessibility_active = true;
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    for _ in 0..3 {
        let previous = prepaints.get();
        cx.update(|window, cx| {
            view.update(cx, |_, cx| cx.notify());
            window.draw(cx).clear(cx);
        });
        assert!(
            prepaints.get() > previous,
            "accessible frames must revisit panel controls"
        );
    }
}

#[gpui::test]
fn accessibility_redraw_releases_the_mutated_view_before_rendering(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    let prepaints = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        panel: cx.new(|_| Panel {
            prepaints: prepaints.clone(),
        }),
        accessibility_active: true,
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let before = prepaints.get();

    cx.update(|window, cx| {
        view.update(cx, |_, cx| {
            cx.notify();
            defer_accessibility_draw(window, cx);
            assert_eq!(
                prepaints.get(),
                before,
                "must not draw inside the view lease"
            );
        });
    });
    assert!(
        prepaints.get() > before,
        "must draw after the action finishes"
    );
}
