use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, px,
};

use crate::app::{
    FarcasterApp,
    ui::theme::theme,
    views::attachments::{image_card, open_card},
};
use crate::conversation::TranscriptItem;

pub(crate) const ATTACHMENT_ROW_HEIGHT: Pixels = px(60.0);

pub(crate) fn render_attachments(
    key: usize,
    item: &TranscriptItem,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    div()
        .id(("message-attachments", key))
        .w_full()
        .mb(theme().space.sm)
        .flex()
        .gap(theme().space.xs)
        .overflow_x_scroll()
        .pb(theme().space.xs)
        .children(item.images.iter().enumerate().map(|(index, image)| {
            let image = crate::app::ui::images::image(image);
            image_card(
                format!("message-image-{key}-{index}"),
                image,
                index,
                item.images.len(),
                entity.clone(),
            )
        }))
        .children(item.files.iter().enumerate().map(|(index, file)| {
            let open = entity.clone();
            let path = file.path.clone();
            open_card(
                format!("message-file-{key}-{index}"),
                file.name.clone(),
                "Text file".into(),
                None,
                move |window, cx| {
                    let _ = open.update(cx, |this, cx| {
                        this.open_file_editor(path.clone(), window, cx);
                    });
                },
            )
        }))
        .into_any_element()
}
