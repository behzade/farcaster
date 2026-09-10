use super::*;

#[gpui::test]
fn header_toggle_and_neovim_action_are_independent(cx: &mut gpui::TestAppContext) {
    use gpui::{FocusHandle, Render, point, px};
    use std::{cell::Cell, rc::Rc};

    struct Header {
        owner: FocusHandle,
        expanded: Rc<Cell<bool>>,
        opened: Rc<Cell<usize>>,
    }
    impl Render for Header {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let expanded = self.expanded.clone();
            let opened = self.opened.clone();
            div()
                .track_focus(&self.owner)
                .capture_key_down(|event, window, cx| {
                    crate::app::ui::focus::traverse_tab(event, None, window, cx);
                })
                .w(px(700.0))
                .child(review_header(
                    7,
                    "Review cancellation".into(),
                    7,
                    expanded.get(),
                    move |_, cx| {
                        expanded.set(!expanded.get());
                        cx.refresh_windows();
                    },
                    move |_, _| opened.set(opened.get() + 1),
                ))
        }
    }
    cx.update(gpui_component::init);
    let expanded = Rc::new(Cell::new(false));
    let opened = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        Header {
            owner,
            expanded: expanded.clone(),
            opened: opened.clone(),
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let header = cx.debug_bounds("review-header-7").unwrap();
    let icon = cx.debug_bounds("review-open-7").unwrap();
    // The trailing blank part of the row toggles too; the icon stays beside
    // the title/count rather than being pushed to the far edge.
    assert!(icon.right() < header.right() - px(40.0));
    cx.simulate_click(
        point(header.right() - px(5.0), header.center().y),
        Default::default(),
    );
    assert!(expanded.get());
    assert_eq!(opened.get(), 0);
    cx.simulate_click(icon.center(), Default::default());
    assert!(expanded.get());
    assert_eq!(opened.get(), 1);
    // Pointer activation preserves the keyboard owner. Tab visits the row,
    // then its independent editor control.
    cx.simulate_keystrokes("tab enter");
    assert!(!expanded.get());
    assert_eq!(opened.get(), 1);
    cx.simulate_keystrokes("tab enter");
    assert!(!expanded.get());
    assert_eq!(opened.get(), 2);
    cx.simulate_keystrokes("space");
    assert!(!expanded.get());
    assert_eq!(opened.get(), 3);
}
