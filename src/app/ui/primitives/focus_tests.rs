use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, point, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{ButtonTone, dropdown_button, preserve_pointer_focus};

struct Controls {
    owner: FocusHandle,
    toggled: bool,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.owner)
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("toggle")
                    .tab_index(0)
                    .w(px(100.0))
                    .h(px(40.0))
                    .on_mouse_down(gpui::MouseButton::Left, preserve_pointer_focus)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggled = !this.toggled;
                        cx.notify();
                    }))
                    .child("Toggle"),
            )
            .child(
                dropdown_button("menu", "Menu", ButtonTone::Neutral, true)
                    .w(px(100.0))
                    .h(px(40.0))
                    .dropdown_menu(|menu, _, _| {
                        menu.item(PopupMenuItem::new("Choose").on_click(|_, _, _| {}))
                    }),
            )
    }
}

#[gpui::test]
fn pointer_controls_and_dropdowns_restore_the_keyboard_owner(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        Controls {
            owner,
            toggled: false,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_click(point(px(20.0), px(20.0)), Default::default());
    cx.update(|window, cx| {
        assert!(view.read(cx).toggled);
        assert!(view.read(cx).owner.is_focused(window));
    });

    for keys in ["escape", "down enter"] {
        cx.simulate_click(point(px(20.0), px(60.0)), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| assert!(!view.read(cx).owner.is_focused(window)));
        cx.simulate_keystrokes(keys);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| assert!(view.read(cx).owner.is_focused(window)));
    }
    // Pointer protection must not remove controls from deliberate Tab traversal.
    cx.update(|window, cx| window.focus_next(cx));
    cx.update(|window, cx| assert!(!view.read(cx).owner.is_focused(window)));
}
