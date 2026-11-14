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
    views::attachments::{image_card, open_card},
};
use crate::conversation::TranscriptItem;

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
        .flex_wrap()
        .items_start()
        .gap(theme().space.sm)
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
        }))
        .into_any_element()
}
