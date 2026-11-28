use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _,
    WeakEntity, div,
};

use crate::app::{
    FarcasterApp,
    attachment::{self, AttachmentSource},
    ui::{
        primitives::{AppTooltip as _, ContextMenuExt as _, ContextMenuTrigger, DropdownMenu as _},
        theme::theme,
    },
    views::attachments::{image_card, image_thumbnail, open_card},
};
use crate::conversation::{TranscriptItem, TranscriptKind};

pub(crate) fn render_attachments(
    key: usize,
    item: &TranscriptItem,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    render_group_attachments(key, std::iter::once((key, item)), entity)
}

pub(crate) fn render_group_attachments<'a>(
    key: usize,
    items: impl Iterator<Item = (usize, &'a TranscriptItem)>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let items: Vec<_> = items.filter(|(_, item)| item.has_attachments()).collect();
    let gallery: Vec<_> = items
        .iter()
        .flat_map(|(_, item)| item.images.iter())
        .map(crate::app::ui::images::image)
        .collect();
    let image_count = gallery.len();
    let mut image_offset = 0;
    div()
        .id(("message-attachments", key))
        .w_full()
        .mb(theme().space.sm)
        .flex()
        .flex_wrap()
        .items_start()
        .gap(theme().space.sm)
        .children(items.into_iter().flat_map(|(source_key, item)| {
            let cards = attachment_cards(
                source_key,
                item,
                image_offset,
                image_count,
                gallery.clone(),
                entity.clone(),
            );
            image_offset += item.images.len();
            cards
        }))
        .into_any_element()
}

fn attachment_cards(
    key: usize,
    item: &TranscriptItem,
    image_offset: usize,
    image_count: usize,
    gallery: Vec<std::sync::Arc<gpui::Image>>,
    entity: WeakEntity<FarcasterApp>,
) -> Vec<AnyElement> {
    item.images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            let image = crate::app::ui::images::image(image);
            let id = format!("message-image-{key}-{index}");
            if item.kind == TranscriptKind::Tool {
                let open = entity.clone();
                let gallery = gallery.clone();
                image_thumbnail(id.clone(), image, image_caption(item), move |window, cx| {
                    let _ = open.update(cx, |this, cx| {
                        this.open_image_gallery(gallery.clone(), image_offset + index, window, cx);
                    });
                })
                .debug_selector(move || id.clone())
                .into_any_element()
            } else {
                image_card(
                    id,
                    image,
                    image_offset + index,
                    image_count,
                    Some(gallery.clone()),
                    entity.clone(),
                )
                .into_any_element()
            }
        })
        .chain(item.files.iter().enumerate().map(|(index, file)| {
            let local = crate::app::workspace::files::local_file_target(
                &file.path.to_string_lossy(),
                std::path::Path::new("/"),
            );
            let rule = attachment::resolve(match &local {
                Some((path, _)) => AttachmentSource::File(path),
                None => AttachmentSource::Url(file.path.to_str().unwrap_or_default()),
            });
            let open = entity.clone();
            let path = file.path.clone();
            let edit = entity.clone();
            let reveal_path = local.map(|target| target.0);
            let group = format!("file-attachment-{key}-{index}");
            let card = open_card(
                format!("message-file-{key}-{index}"),
                file.name.clone(),
                String::new(),
                None,
                move |window, cx| {
                    let _ = open.update(cx, |this, cx| {
                        this.open_transcript_link(&path.to_string_lossy(), window, cx);
                    });
                },
            );
            let menu_entity = entity.clone();
            let menu_path = file.path.clone();
            let actions = std::rc::Rc::new(
                move |menu: gpui_component::menu::PopupMenu,
                      _: &mut gpui::Window,
                      _: &mut gpui::Context<gpui_component::menu::PopupMenu>| {
                    use gpui_component::menu::PopupMenuItem;
                    let open = menu_entity.clone();
                    let path = menu_path.clone();
                    let mut menu =
                        menu.item(PopupMenuItem::new("Open").on_click(move |_, window, cx| {
                            let _ = open.update(cx, |this, cx| {
                                this.open_transcript_link(&path.to_string_lossy(), window, cx)
                            });
                        }));
                    if let (Some(source), Some(action)) = (&reveal_path, rule.source) {
                        let edit = edit.clone();
                        let source = source.clone();
                        menu = menu.item(PopupMenuItem::new("Edit source").on_click(
                            move |_, window, cx| {
                                let _ = edit.update(cx, |this, cx| {
                                    this.open_attachment_action(
                                        source.clone(),
                                        None,
                                        action,
                                        window,
                                        cx,
                                    )
                                });
                            },
                        ));
                    }
                    if let Some(path) = reveal_path.clone() {
                        menu = menu.item(
                            PopupMenuItem::new("Reveal file")
                                .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        );
                    }
                    menu
                },
            );
            let dropdown_actions = actions.clone();
            use gpui_component::{
                Sizable as _, Size,
                button::{Button, ButtonVariants as _},
            };
            let card = card.group(group.clone()).pr(theme().size(24.0)).child(
                Button::new("attachment-actions")
                    .label("…")
                    .ghost()
                    .with_size(Size::XSmall)
                    .absolute()
                    .top_0()
                    .right_0()
                    .opacity(0.0)
                    .group_hover(group, |button| button.opacity(1.0))
                    .focus_visible(|button| button.opacity(1.0))
                    .accessibility_label("Attachment actions")
                    .app_tooltip("Attachment actions")
                    .dropdown_menu(move |menu, window, cx| dropdown_actions(menu, window, cx)),
            );
            ContextMenuTrigger::new(
                format!("attachment-menu-{key}-{index}"),
                card.into_any_element(),
            )
            .w_auto()
            .max_w_full()
            .context_menu(move |menu, window, cx| actions(menu, window, cx))
            .into_any_element()
        }))
        .collect()
}

fn image_caption(item: &TranscriptItem) -> String {
    let Some(details) = &item.tool_details else {
        return item.label.clone();
    };
    let title = details
        .arguments
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            details
                .metadata
                .title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty() && *title != details.name)
        });
    if let Some(title) = title {
        return title.to_owned();
    }
    details
        .metadata
        .targets
        .first()
        .map(String::as_str)
        .or_else(|| {
            details
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
        })
        .and_then(|path| std::path::Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| details.summary())
}

#[cfg(test)]
#[path = "attachments_tests.rs"]
mod tests;
