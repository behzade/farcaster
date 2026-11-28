use super::*;
use gpui::{Context, InteractiveElement as _, Render, Window, px};
use std::{cell::Cell, rc::Rc};

struct AttachmentsHarness {
    clicks: Rc<Cell<usize>>,
    owner: gpui::FocusHandle,
    mixed: bool,
}

impl Render for AttachmentsHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .track_focus(&self.owner)
            .capture_key_down(|event, window, cx| {
                crate::app::ui::focus::traverse_tab(event, None, window, cx);
            })
            .w(if self.mixed { px(700.) } else { px(260.) })
            .flex()
            .flex_wrap()
            .gap(theme().space.md)
            .children((0usize..3).map(|index| {
                let clicks = self.clicks.clone();
                open_card(
                    ("file", index),
                    if self.mixed {
                        ["Image", "notes.md", "Pasted text"][index]
                    } else if index == 2 {
                        "notes.md"
                    } else {
                        "a very long presentation filename.html"
                    }
                    .into(),
                    if self.mixed {
                        ["PNG · 1 KB", "", "3 lines · 113 B"][index].into()
                    } else {
                        String::new()
                    },
                    (self.mixed && index == 0).then(|| {
                        Arc::new(Image::from_bytes(
                            gpui::ImageFormat::Png,
                            include_bytes!("../../../assets/icons/app/icon_16x16.png").to_vec(),
                        ))
                    }),
                    move |_, _| clicks.set(clicks.get() + 1),
                )
                .debug_selector(move || format!("attachment-{index}"))
            }))
    }
}

#[gpui::test]
fn attachments_wrap_and_open_with_keyboard(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let clicks = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        AttachmentsHarness {
            clicks: clicks.clone(),
            owner,
            mixed: false,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let first = cx.debug_bounds("attachment-0").unwrap();
    let second = cx.debug_bounds("attachment-1").unwrap();
    let third = cx.debug_bounds("attachment-2").unwrap();
    for bounds in [first, second, third] {
        assert!(bounds.right() <= px(260.), "attachment must fit the row");
    }
    assert!(second.top() >= first.bottom(), "second item wraps");
    assert!(third.top() >= second.bottom(), "third item wraps");
    assert!(
        third.size.width < first.size.width,
        "short names do not leave a wide empty slot"
    );
    cx.simulate_keystrokes("tab enter");
    assert_eq!(clicks.get(), 1, "keyboard activates the attachment");
}

#[gpui::test]
fn images_files_and_pastes_share_height_and_top_edge(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        AttachmentsHarness {
            clicks: Rc::new(Cell::new(0)),
            owner,
            mixed: true,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let image = cx.debug_bounds("attachment-0").unwrap();
    for selector in ["attachment-0", "attachment-1", "attachment-2"] {
        let bounds = cx.debug_bounds(selector).unwrap();
        assert_eq!(
            bounds.top(),
            image.top(),
            "all attachment types align at the top"
        );
        assert_eq!(
            bounds.size.height,
            theme().size(40.0),
            "all attachment types have the same compact height"
        );
    }
}

#[gpui::test]
fn tool_thumbnails_support_keyboard_and_mouse(cx: &mut gpui::TestAppContext) {
    struct Thumbnail {
        clicks: Rc<Cell<usize>>,
        owner: gpui::FocusHandle,
    }
    impl Render for Thumbnail {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            let clicks = self.clicks.clone();
            div()
                .track_focus(&self.owner)
                .capture_key_down(|event, window, cx| {
                    crate::app::ui::focus::traverse_tab(event, None, window, cx);
                })
                .child(
                    image_thumbnail(
                        "screenshot",
                        Arc::new(Image::from_bytes(
                            gpui::ImageFormat::Png,
                            include_bytes!("../../../assets/icons/app/icon_16x16.png").to_vec(),
                        )),
                        "Inspect appearance settings".into(),
                        move |_, _| clicks.set(clicks.get() + 1),
                    )
                    .debug_selector(|| "screenshot".into()),
                )
        }
    }
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::ui::primitives::init_tooltips(cx);
    });
    let clicks = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let owner = cx.focus_handle();
        owner.focus(window, cx);
        Thumbnail {
            clicks: clicks.clone(),
            owner,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("tab enter");
    assert_eq!(clicks.get(), 1);
    let bounds = cx.debug_bounds("screenshot").unwrap();
    cx.simulate_click(bounds.center(), Default::default());
    assert_eq!(clicks.get(), 2);
}
