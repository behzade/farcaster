use super::*;
use gpui::{Context, FocusHandle, MouseButton, Render, Window, point, px};
use gpui_component::menu::{ContextMenuExt as _, PopupMenuItem};
use std::{cell::Cell, rc::Rc};

struct MenuHarness {
    owner: FocusHandle,
    clicks: Rc<Cell<usize>>,
}

impl Render for MenuHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        div().size_full().track_focus(&self.owner).child(
            ContextMenuTrigger::new("context", div().child("Right click").into_any_element())
                .w(px(120.))
                .h(px(40.))
                .context_menu(move |menu, window, cx| {
                    let clicks = clicks.clone();
                    menu.submenu("More", window, cx, move |menu, _, _| {
                        let clicks = clicks.clone();
                        menu.item(
                            PopupMenuItem::element(|_, _| {
                                div()
                                    .debug_selector(|| "nested-menu-item".into())
                                    .child("Choose")
                            })
                            .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                        )
                    })
                }),
        )
    }
}

#[gpui::test]
fn context_menu_handles_overhanging_submenu_and_restores_focus(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let clicks = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        MenuHarness {
            owner,
            clicks: clicks.clone(),
        }
    });
    let open = |cx: &mut gpui::VisualTestContext| {
        let pointer = point(px(50.), px(20.));
        cx.simulate_mouse_down(pointer, MouseButton::Right, Default::default());
        cx.simulate_mouse_up(pointer, MouseButton::Right, Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
    };
    open(cx);
    cx.simulate_keystrokes("down right");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let item = cx.debug_bounds("nested-menu-item").expect("submenu opened");
    assert!(item.left() > px(120.), "submenu extends past the trigger");
    cx.simulate_click(item.center(), Default::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert_eq!(clicks.get(), 1);
        assert!(view.read(cx).owner.is_focused(window));
    });
    open(cx);
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| assert!(view.read(cx).owner.is_focused(window)));
    open(cx);
    cx.simulate_keystrokes("down right");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_click(point(px(600.), px(400.)), Default::default());
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(view.read(cx).owner.is_focused(window));
    });
    assert!(cx.debug_bounds("nested-menu-item").is_none());
}
