use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, WeakEntity, div,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use super::super::{
    FarcasterApp,
    attachments::{format_bytes, image_card, open_card},
};
use crate::app::ui::theme::THEME;

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> Option<AnyElement> {
    let images = app.current_composer_images();
    let pastes = app.current_composer_pastes();
    if images.is_empty() && pastes.is_empty() {
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
                image_card(
                    ("composer-image", index),
                    image.preview.clone(),
                    index,
                    images.len(),
                    entity.clone(),
                )
                .child(remove_button(
                    ("remove-composer-image", index),
                    "Remove image",
                    move |cx| {
                        let _ = remove.update(cx, |this, cx| this.remove_composer_image(index, cx));
                    },
                ))
            }))
            .children(pastes.iter().enumerate().map(|(index, paste)| {
                let open = entity.clone();
                let remove = entity.clone();
                open_card(
                    ("composer-paste", index),
                    "Pasted text".into(),
                    format!(
                        "{} lines · {}",
                        paste.line_count,
                        format_bytes(paste.content.len())
                    ),
                    None,
                    move |window, cx| {
                        let _ =
                            open.update(cx, |this, cx| this.open_composer_paste(index, window, cx));
                    },
                )
                .child(remove_button(
                    ("remove-composer-paste", index),
                    "Remove pasted text",
                    move |cx| {
                        let _ = remove.update(cx, |this, cx| this.remove_composer_paste(index, cx));
                    },
                ))
            }))
            .into_any_element(),
    )
}

fn remove_button(
    id: impl Into<ElementId>,
    tooltip: &'static str,
    remove: impl Fn(&mut App) + 'static,
) -> Button {
    Button::new(id)
        .label("×")
        .tooltip(tooltip)
        .with_size(Size::XSmall)
        .ghost()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            remove(cx);
        })
}
