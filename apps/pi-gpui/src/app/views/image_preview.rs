//! Full-size transcript image preview.

use gpui::{
    AnyElement, IntoElement as _, ObjectFit, ParentElement as _, Styled as _, StyledImage as _,
    WeakEntity, div, img, px, relative,
};

use super::super::PiApp;
use crate::{
    assets::AppIcon,
    primitives::{ButtonTone, icon_button, modal},
    theme::THEME,
};

pub(super) fn render(app: &PiApp, entity: WeakEntity<PiApp>) -> Option<AnyElement> {
    let preview = app.image_preview.as_ref()?.clone();
    let close = entity.clone();
    Some(
        modal(
            "image-preview",
            "Image preview",
            &app.image_preview_focus,
            super::OVERLAY_KEY_CONTEXT,
            move |window, cx| {
                let _ = close.update(cx, |this, cx| this.close_image_preview(window, cx));
            },
            |surface| {
                let close = entity.clone();
                surface
                    .w(px(960.0))
                    .max_w_full()
                    .h(relative(0.86))
                    .max_h(relative(0.92))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(48.0))
                            .flex_none()
                            .px(THEME.space.md)
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b(THEME.border)
                            .border_color(THEME.colors.border)
                            .child(format!(
                                "Attachment {} of {}",
                                preview.index + 1,
                                preview.total
                            ))
                            .child(icon_button(
                                "close-image-preview",
                                AppIcon::X,
                                "Close image preview",
                                ButtonTone::Quiet,
                                move |window, cx| {
                                    let _ = close.update(cx, |this, cx| {
                                        this.close_image_preview(window, cx);
                                    });
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .p(THEME.space.md)
                            .bg(THEME.colors.canvas)
                            .child(
                                img(preview.image)
                                    .size_full()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    )
            },
        )
        .into_any_element(),
    )
}
