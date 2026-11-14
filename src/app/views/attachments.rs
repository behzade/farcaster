use std::sync::Arc;

use crate::app::{
    FarcasterApp,
    attachment::{self, AttachmentSource, AttachmentView},
    ui::theme::{UI_FONT_FAMILY, theme},
};
use gpui::{
    AnyElement, Div, Image, IntoElement as _, ObjectFit, ParentElement as _, Styled as _,
    StyledImage as _, WeakEntity, div, img, prelude::FluentBuilder as _,
};

fn card() -> Div {
    div()
        .relative()
        .flex_none()
        .max_w_full()
        .font_family(UI_FONT_FAMILY)
        .text_size(theme().type_scale.caption)
        .text_color(theme().colors.text)
}

fn content(name: String, detail: String, image: Option<Arc<Image>>) -> AnyElement {
    let rule = attachment::resolve(match &image {
        Some(image) => AttachmentSource::Image(image.format()),
        None => AttachmentSource::File(std::path::Path::new(&name)),
    });
    let preview = match image {
        Some(image) => img(image)
            .size(theme().size(24.0))
            .flex_none()
            .rounded(theme().size(3.0))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => crate::app::ui::file_icons::file_icon(std::path::Path::new(&name)),
    };
    let (label, detail) = if rule.view == AttachmentView::Image {
        (detail, String::new())
    } else {
        (name, detail)
    };
    div()
        .h_full()
        .max_w(theme().size(220.0))
        .min_w_0()
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .px(theme().space.sm)
        .child(preview)
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().child(label))
                .when(!detail.is_empty(), |text| {
                    text.child(
                        div()
                            .truncate()
                            .text_color(theme().colors.muted)
                            .child(detail),
                    )
                }),
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
    card()
        .id(id)
        .h(theme().size(40.0))
        .rounded(theme().size(6.0))
        .bg(theme().colors.surface)
        .child(
            div()
                .id("open-attachment")
                .h_full()
                .max_w_full()
                .border(theme().border)
                .border_color(gpui::transparent_black())
                .cursor_pointer()
                .tab_index(0)
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    crate::app::ui::primitives::preserve_pointer_focus,
                )
                .role(gpui::Role::Button)
                .aria_label(if detail.is_empty() {
                    format!("Open {name}")
                } else {
                    format!("{name}: {detail}")
                })
                .hover(|control| control.text_color(theme().colors.accent))
                .focus_visible(|control| control.border_color(theme().colors.focus_border))
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

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
