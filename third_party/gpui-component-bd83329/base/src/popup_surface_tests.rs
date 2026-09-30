use super::*;
use gpui::{Context, Render, point};
use std::cell::RefCell;

struct Harness {
    clicks: Rc<Cell<usize>>,
    nested_open: bool,
    changes: Rc<RefCell<Vec<bool>>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        let nested_open = self.nested_open;
        let changes = self.changes.clone();
        crate::Popover::new("parent")
            .default_open(true)
            .trigger_with(|_, _, _| div().size(px(80.)).into_any_element())
            .content(move |_, _, _| {
                div()
                    .size(px(80.))
                    .debug_selector(|| "parent-surface".into())
                    .child({
                        let popup = Popup::new("nested", div().size(px(20.)))
                            .position(point(px(160.), px(100.)));
                        if nested_open {
                            popup.content(
                                div()
                                    .id("nested-content")
                                    .size(px(40.))
                                    .occlude()
                                    .debug_selector(|| "nested-surface".into())
                                    .on_any_mouse_down(move |_, _, _| {
                                        clicks.set(clicks.get() + 1);
                                    }),
                            )
                        } else {
                            popup
                        }
                    })
            })
            .on_open_change(move |open, _, _| changes.borrow_mut().push(*open))
    }
}

#[gpui::test]
fn clicking_an_overhanging_popup_keeps_its_parent_open(cx: &mut gpui::TestAppContext) {
    cx.update(crate::init);
    let clicks = Rc::new(Cell::new(0));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let (_, cx) = cx.add_window_view({
        let clicks = clicks.clone();
        let changes = changes.clone();
        move |_, _| Harness {
            clicks,
            nested_open: true,
            changes,
        }
    });
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    let pointer = cx
        .debug_bounds("nested-surface")
        .expect("nested popup")
        .center();
    assert!(
        !cx.debug_bounds("parent-surface")
            .unwrap()
            .contains(&pointer)
    );
    cx.simulate_click(pointer, Default::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(clicks.get(), 1);
    assert!(changes.borrow().is_empty());
    assert!(cx.debug_bounds("parent-surface").is_some());

    cx.simulate_click(point(px(300.), px(300.)), Default::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("parent-surface").is_none());
    assert!(changes.borrow().contains(&false));
}

#[gpui::test]
fn a_removed_popup_does_not_claim_clicks_in_its_old_bounds(cx: &mut gpui::TestAppContext) {
    cx.update(crate::init);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let (view, cx) = cx.add_window_view({
        let changes = changes.clone();
        move |_, _| Harness {
            clicks: Rc::new(Cell::new(0)),
            nested_open: true,
            changes,
        }
    });
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    let old_pointer = cx.debug_bounds("nested-surface").unwrap().center();
    view.update(cx, |view, cx| {
        view.nested_open = false;
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("nested-surface").is_none());
    cx.simulate_click(old_pointer, Default::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("parent-surface").is_none());
    assert!(changes.borrow().contains(&false));
}

#[gpui::test]
fn another_windows_popup_does_not_prevent_outside_dismissal(cx: &mut gpui::TestAppContext) {
    cx.update(crate::init);
    let pointer = {
        let (_, other_window) = cx.add_window_view(|_, _| Harness {
            clicks: Rc::new(Cell::new(0)),
            nested_open: true,
            changes: Rc::new(RefCell::new(Vec::new())),
        });
        for _ in 0..3 {
            other_window.update(|window, cx| window.draw(cx).clear(cx));
        }
        other_window
            .debug_bounds("nested-surface")
            .unwrap()
            .center()
    };
    let (_, this_window) = cx.add_window_view(|_, _| Harness {
        clicks: Rc::new(Cell::new(0)),
        nested_open: false,
        changes: Rc::new(RefCell::new(Vec::new())),
    });
    for _ in 0..2 {
        this_window.update(|window, cx| window.draw(cx).clear(cx));
    }
    this_window.simulate_click(pointer, Default::default());
    this_window.update(|window, cx| window.draw(cx).clear(cx));
    assert!(this_window.debug_bounds("parent-surface").is_none());
}
