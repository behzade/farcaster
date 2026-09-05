//! Shared attachment chrome for drafts and transcript history.
use std::sync::Arc;

use crate::app::{
    FarcasterApp,
    ui::theme::{THEME, UI_FONT_FAMILY},
};
use gpui::{
    AnyElement, Div, Image, IntoElement as _, ObjectFit, ParentElement as _, Styled as _,
    StyledImage as _, WeakEntity, div, img, px,
};

fn card() -> Div {
    div()
        .h(px(48.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .px(THEME.space.sm)
        .rounded(THEME.radius)
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.surface)
        .font_family(UI_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.text)
}

fn content(name: String, detail: String, image: Option<Arc<Image>>) -> AnyElement {
    let preview = match image {
        Some(image) => img(image)
            .size(px(32.0))
            .rounded(px(3.0))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => div()
            .size(px(32.0))
            .flex()
            .items_center()
            .justify_center()
            .text_color(THEME.colors.muted)
            .child("TXT")
            .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .child(preview)
        .child(
            div()
                .flex()
                .flex_col()
                .max_w(px(220.0))
                .child(div().truncate().child(name))
                .child(div().text_color(THEME.colors.muted).child(detail)),
        )
        .into_any_element()
}

pub(super) fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn open_card(
    id: impl Into<gpui::ElementId>,
    name: String,
    detail: String,
    image: Option<Arc<Image>>,
    open: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<Div> {
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    let open = std::rc::Rc::new(open);
    let click = open.clone();
    // Keep opening and removal as sibling controls, not nested buttons.
    card()
        .id(id)
        .hover(|card| card.border_color(THEME.colors.accent))
        .child(
            div()
                .id("open-attachment")
                .h_full()
                .flex()
                .items_center()
                .rounded(THEME.radius)
                .border(THEME.border)
                .border_color(THEME.colors.surface)
                .cursor_pointer()
                .tab_index(0)
                .role(gpui::Role::Button)
                .aria_label(format!("Open {name}"))
                .focus_visible(|control| control.border_color(THEME.colors.accent))
                .child(content(name, detail, image))
                .on_click(move |_, window, cx| click(window, cx))
                .on_key_down(move |event, window, cx| {
                    if crate::app::ui::primitives::activates_button(event) {
                        cx.stop_propagation();
                        open(window, cx);
                    }
                }),
        )
}

/// Image labels and preview actions are identical in drafts and history.
pub(super) fn image_card(
    id: impl Into<gpui::ElementId>,
    image: Arc<Image>,
    index: usize,
    count: usize,
    entity: WeakEntity<FarcasterApp>,
) -> gpui::Stateful<Div> {
    let format = image
        .format()
        .mime_type()
        .strip_prefix("image/")
        .unwrap_or("image")
        .to_ascii_uppercase();
    open_card(
        id,
        "Image".into(),
        format!("{format} · {}", format_bytes(image.bytes().len())),
        Some(image.clone()),
        move |window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.open_image_preview(image.clone(), index, count, window, cx);
            });
        },
    )
}
