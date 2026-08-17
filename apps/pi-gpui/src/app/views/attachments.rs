//! Compact composer attachment chips.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ObjectFit, ParentElement as _,
    Styled as _, StyledImage as _, WeakEntity, div, img, px,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use super::super::PiApp;
use crate::theme::{MONO_FONT_FAMILY, THEME};

pub(super) fn render(app: &PiApp, entity: WeakEntity<PiApp>) -> Option<AnyElement> {
    let images = app.current_composer_images();
    if images.is_empty() {
        return None;
    }
    Some(
        div()
            .id("composer-attachments")
            .px(THEME.space.sm)
            .pb(THEME.space.xs)
            .flex()
            .flex_wrap()
            .gap(THEME.space.xs)
            .children(images.iter().enumerate().map(|(index, image)| {
                let remove = entity.clone();
                let format = image
                    .prompt
                    .mime_type
                    .strip_prefix("image/")
                    .unwrap_or(&image.prompt.mime_type)
                    .to_ascii_uppercase();
                div()
                    .id(("composer-image", index))
                    .h(px(36.0))
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .pl(THEME.space.sm)
                    .pr(THEME.space.xs)
                    .rounded(THEME.radius)
                    .border(THEME.border)
                    .border_color(THEME.colors.border)
                    .bg(THEME.colors.surface)
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(
                        img(image.preview.clone())
                            .size(px(24.0))
                            .rounded(px(3.0))
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(format!(
                        "Image · {format} · {}",
                        format_bytes(image.byte_len)
                    ))
                    .child(
                        Button::new(("remove-composer-image", index))
                            .label("×")
                            .tooltip("Remove image")
                            .with_size(Size::XSmall)
                            .ghost()
                            .on_click(move |_, _, cx| {
                                let _ = remove.update(cx, |this, cx| {
                                    this.remove_composer_image(index, cx);
                                });
                            }),
                    )
            }))
            .into_any_element(),
    )
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
