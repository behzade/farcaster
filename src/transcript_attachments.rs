//! User-message image attachment strip.

use std::sync::Arc;

use gpui::{
    AnyElement, App, FontWeight, Image, InteractiveElement as _, IntoElement as _, ObjectFit,
    ParentElement as _, Pixels, Role, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, WeakEntity, Window, div, img, px,
};

use crate::{
    app::FarcasterApp, conversation::TranscriptItem, primitives::activates_button, theme::THEME,
};

pub(crate) const ATTACHMENT_ROW_HEIGHT: Pixels = px(126.0);

fn open_image(
    entity: &WeakEntity<FarcasterApp>,
    image: &Arc<Image>,
    index: usize,
    count: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = entity.update(cx, |this, cx| {
        this.open_image_preview(image.clone(), index, count, window, cx);
    });
}

pub(crate) fn render_attachments(
    key: usize,
    item: &TranscriptItem,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let count = item.images.len();
    div()
        .id(("message-attachments", key))
        .w_full()
        .mb(THEME.space.sm)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_size(THEME.type_scale.caption)
                .font_weight(FontWeight::MEDIUM)
                .text_color(THEME.colors.muted)
                .child(match count {
                    1 => "1 attachment".to_owned(),
                    _ => format!("{count} attachments"),
                }),
        )
        .child(
            div()
                .id(("message-attachment-strip", key))
                .w_full()
                .flex()
                .gap(THEME.space.sm)
                .overflow_x_scroll()
                .pb(THEME.space.xs)
                .children(item.images.iter().enumerate().map(|(index, image)| {
                    let click_entity = entity.clone();
                    let click_image = image.clone();
                    let key_entity = entity.clone();
                    let key_image = image.clone();
                    div()
                        .id(format!("message-attachment-{key}-{index}"))
                        .relative()
                        .w(px(124.0))
                        .h(px(92.0))
                        .flex_none()
                        .overflow_hidden()
                        .rounded(THEME.radius)
                        .border(THEME.border)
                        .border_color(THEME.colors.border)
                        .bg(THEME.colors.surface)
                        .cursor_pointer()
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label(format!("Open attachment {} of {count}", index + 1))
                        .hover(|card| card.border_color(THEME.colors.accent))
                        .focus(|card| card.border_color(THEME.colors.accent))
                        .child(img(image.clone()).size_full().object_fit(ObjectFit::Cover))
                        .child(
                            div()
                                .absolute()
                                .right(THEME.space.xs)
                                .bottom(THEME.space.xs)
                                .px(THEME.space.xs)
                                .py(px(1.0))
                                .rounded(px(3.0))
                                .bg(THEME.colors.panel)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child(format!("{} / {count}", index + 1)),
                        )
                        .on_click(move |_, window, cx| {
                            open_image(&click_entity, &click_image, index, count, window, cx);
                        })
                        .on_key_down(move |event, window, cx| {
                            if activates_button(event) {
                                cx.stop_propagation();
                                open_image(&key_entity, &key_image, index, count, window, cx);
                            }
                        })
                })),
        )
        .into_any_element()
}
