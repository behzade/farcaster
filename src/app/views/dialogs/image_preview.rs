use gpui::{
    AnyElement, IntoElement as _, ObjectFit, ParentElement as _, Styled as _, StyledImage as _,
    WeakEntity, div, img,
};

use crate::app::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{ButtonTone, icon_button, modal},
    app::ui::theme::theme,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> Option<AnyElement> {
    let preview = app.overlays.image_preview.as_ref()?.clone();
    let close = entity.clone();
    Some(
        modal(
            "image-preview",
            format!("Image {} of {}", preview.index + 1, preview.total),
            &app.overlays.image_preview_focus,
            crate::app::OVERLAY_KEY_CONTEXT,
            move |window, cx| {
                let _ = close.update(cx, |this, cx| this.close_image_preview(window, cx));
            },
            |surface| {
                let close = entity.clone();
                surface
                    .relative()
                    .w_full()
                    .h_full()
                    .max_h_full()
                    .rounded_none()
                    .shadow(Vec::new())
                    .bg(gpui::transparent_black())
                    .overflow_hidden()
                    .child(
                        img(preview.rendered.clone())
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(theme().space.sm)
                            .right(theme().space.sm)
                            .flex()
                            .gap(theme().space.xs)
                            .child(icon_button(
                                "open-image-externally",
                                AppIcon::Eye,
                                "Open image externally",
                                ButtonTone::Quiet,
                                {
                                    let entity = entity.clone();
                                    let image = preview.image.clone();
                                    move |window, cx| {
                                        let _ = entity.update(cx, |this, cx| {
                                            this.open_image_externally(image.clone(), window, cx)
                                        });
                                    }
                                },
                            ))
                            .child(icon_button(
                                "close-image-preview",
                                AppIcon::X,
                                "Close image preview",
                                ButtonTone::Quiet,
                                move |window, cx| {
                                    let _ = close.update(cx, |this, cx| {
                                        this.close_image_preview(window, cx)
                                    });
                                },
                            )),
                    )
            },
        )
        .into_any_element(),
    )
}
