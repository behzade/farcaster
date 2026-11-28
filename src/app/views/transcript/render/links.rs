use gpui::{
    ClickEvent, IntoElement as _, MouseButton, ParentElement as _, Styled as _, WeakEntity,
};
use gpui_component::text::TextView;

use crate::app::{
    FarcasterApp,
    attachment::{self, AttachmentSource},
};

pub(super) fn with_file_links(text: TextView, entity: WeakEntity<FarcasterApp>) -> TextView {
    let code_entity = entity.clone();
    text.code_block_actions(move |block, _, _| {
        let mut actions = gpui::div()
            .flex()
            .gap_1()
            .child(super::copy_code::CopyCodeButton::new(block.code()));
        if let Some(rule) = block
            .lang()
            .map(|language| attachment::resolve(AttachmentSource::Code(&language)))
            && let Some(source_action) = rule.source
        {
            for (id, icon, label, action) in [
                (
                    "open-html-browser",
                    crate::app::ui::assets::AppIcon::Eye,
                    "Open HTML in browser",
                    rule.open,
                ),
                (
                    "edit-html-source",
                    crate::app::ui::assets::AppIcon::Code,
                    "Edit HTML source",
                    source_action,
                ),
            ] {
                let entity = code_entity.clone();
                let source = block.code().to_string();
                actions = actions.child(crate::app::ui::primitives::icon_button(
                    id,
                    icon,
                    label,
                    crate::app::ui::primitives::ButtonTone::Quiet,
                    move |window, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.open_html_code(source.clone(), action, window, cx)
                        });
                    },
                ));
            }
        }
        actions.into_any_element()
    })
    .on_link_click(move |url, event, window, cx| {
        let activate = match event {
            ClickEvent::Mouse(click) => {
                matches!(click.up.button, MouseButton::Left | MouseButton::Middle)
            }
            ClickEvent::Keyboard(_) => true,
            ClickEvent::Touch(click) => !click.long_press,
        };
        if !activate || url.is_empty() || url.starts_with('#') {
            return;
        }
        let _ = entity.update(cx, |this, cx| {
            this.open_transcript_link(url, window, cx);
        });
    })
}

pub(super) fn with_transcript_images(
    text: TextView,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> TextView {
    let project = entity
        .upgrade()
        .map(|entity| entity.read(cx).workspace_project())
        .unwrap_or_default();
    with_file_links(text, entity)
        .image_source(move |uri| crate::app::workspace::files::image_source(uri.as_ref(), &project))
}
