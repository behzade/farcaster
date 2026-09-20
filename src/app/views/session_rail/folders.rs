use std::path::PathBuf;

use super::{
    FarcasterApp,
    colors::{ColorTarget, color_menu, palette_color},
    drag::DraggedSession,
    groups::{ActiveSessionItem, SessionRailKind},
    rendering::session_section_header,
};
use crate::app::{
    session_folders::SessionFolders,
    ui::{
        assets::AppIcon,
        primitives::{
            AppIconSize, ButtonTone, ContextMenuTrigger, DeleteButton, app_icon, button,
            disclosure_button, icon_control,
        },
        theme::theme,
    },
};
use gpui::{
    Anchor, AnyElement, Entity, InteractiveElement as _, IntoElement as _, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
};
use gpui_component::{
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};

#[derive(Clone)]
pub(super) struct FolderHeader {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) color: u8,
    pub(super) collapsed: bool,
    pub(super) project: Option<PathBuf>,
}

#[derive(Clone)]
pub(super) enum FolderRow {
    Session(Box<ActiveSessionItem>),
    Header(Box<FolderHeader>),
}

pub(super) fn folder_rows(
    items: Vec<ActiveSessionItem>,
    folders: &SessionFolders,
) -> Vec<FolderRow> {
    let mut sections = std::collections::BTreeMap::<Option<u64>, Vec<ActiveSessionItem>>::new();
    for item in items {
        sections
            .entry(folders.folder_for_session(item.app_session_id(), item.project()))
            .or_default()
            .push(item);
    }
    let mut rows = sections
        .remove(&None)
        .unwrap_or_default()
        .into_iter()
        .map(|item| FolderRow::Session(Box::new(item)))
        .collect::<Vec<_>>();
    for folder in &folders.folders {
        let members = sections.remove(&Some(folder.id)).unwrap_or_default();
        rows.push(FolderRow::Header(Box::new(FolderHeader {
            id: folder.id,
            name: folder.name.clone(),
            color: folder.color,
            collapsed: folder.collapsed,
            project: folder.project.clone(),
        })));
        if !folder.collapsed {
            rows.extend(
                members
                    .into_iter()
                    .map(|item| FolderRow::Session(Box::new(item))),
            );
        }
    }
    rows
}

pub(super) fn folder_header(
    folder: FolderHeader,
    editing: bool,
    input: Entity<InputState>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let FolderHeader {
        id,
        name,
        color,
        collapsed,
        project,
    } = folder;
    let drop_entity = entity.clone();
    let edit_entity = entity.clone();
    let new_entity = entity.clone();
    let context_entity = entity.clone();
    let toggle_entity = entity.clone();
    let scope_entity = entity.clone();
    let delete_entity = entity.clone();
    let scope_project = project.clone();
    let cancel_entity = entity;
    let section = div().w_full().flex().flex_col();
    let mut row = session_section_header()
        .id(format!("session-folder-{id}"))
        .group("session-folder-header")
        .w_full()
        .pr(theme().space.sm)
        .cursor_pointer()
        .hover(|row| row.bg(theme().colors.highlight))
        .on_click(move |_, _, cx| {
            let Some(project) = scope_project.clone() else {
                return;
            };
            let _ = scope_entity.update(cx, |this, cx| {
                if this.project.path != project {
                    this.select_project(project.clone(), cx);
                }
            });
        });
    if editing {
        let commit = edit_entity.clone();
        return section
            .child(
                row.on_action(move |_: &gpui_component::input::Escape, _, cx| {
                    cx.stop_propagation();
                    let _ = cancel_entity.update(cx, |this, cx| this.cancel_session_title_edit(cx));
                })
                .child(Input::new(&input).flex_1().min_w_0().appearance(true))
                .child(button(
                    "save-folder",
                    "Save",
                    ButtonTone::Neutral,
                    true,
                    move |_, cx| {
                        let _ = commit.update(cx, |this, cx| this.commit_folder_edit(cx));
                    },
                )),
            )
            .into_any_element();
    }

    row = folder_drop_target(row, move |drag, window, cx| {
        let _ = drop_entity.update(cx, |this, cx| {
            if drag.kind == SessionRailKind::Archived {
                this.request_chat_archive(drag.app_session_id, false, window, cx);
            }
            this.assign_session_folder(drag.app_session_id, Some(id), cx);
            this.clear_session_drop_target(cx);
        });
    });
    row = row
        .child(disclosure_button(
            format!("folder-toggle-{id}"),
            !collapsed,
            "folder",
            move |_, cx| {
                let _ = toggle_entity.update(cx, |this, cx| {
                    this.set_folder_collapsed(id, !collapsed, cx);
                });
            },
        ))
        .child(
            div()
                .flex_none()
                .text_color(palette_color(color))
                .child(app_icon(AppIcon::Folder, AppIconSize::Control)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name),
        );

    row = row.child(
        DeleteButton::new(format!("delete-folder-{id}"), "Delete folder and its chats")
            .reveal_on("session-folder-header")
            .on_delete(move |window, cx| {
                let _ = delete_entity.update(cx, |this, cx| {
                    this.request_folder_delete(id, window, cx);
                });
            }),
    );
    row = row.child(
        div()
            .w(theme().layout.session_age_slot)
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .child(
                icon_control(
                    format!("new-session-in-folder-{id}"),
                    "New session in folder",
                )
                .opacity(0.0)
                .group_hover("session-folder-header", |style| style.opacity(1.0))
                .focus(|style| style.opacity(1.0))
                .text_color(theme().colors.muted)
                .hover(|control| control.bg(theme().colors.highlight))
                .child(app_icon(AppIcon::Plus, AppIconSize::Inline))
                .on_click(move |_, window, cx| {
                    let _ = new_entity.update(cx, |this, cx| {
                        let project = project.clone().unwrap_or_else(|| this.project.path.clone());
                        this.new_session_with_folder(project, Some(id), window, cx);
                    });
                }),
            ),
    );
    let header = ContextMenuTrigger::new(format!("folder-context-{id}"), row.into_any_element())
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, window, cx| {
            let rename = context_entity.clone();
            let colour = context_entity.clone();
            let delete = context_entity.clone();
            menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                let _ = rename.update(cx, |this, cx| this.begin_folder_edit(Some(id), window, cx));
            }))
            .submenu("Colour", window, cx, move |menu, _, _| {
                color_menu(menu, Some(color), colour.clone(), ColorTarget::Folder(id))
            })
            .separator()
            .item(
                PopupMenuItem::new("Delete folder").on_click(move |_, _, cx| {
                    let _ = delete.update(cx, |this, cx| {
                        let mut next = this.sessions.folders.clone();
                        next.remove(id);
                        this.save_session_folders(next, cx);
                    });
                }),
            )
        })
        .mouse_button(MouseButton::Right);
    section.child(header).into_any_element()
}

pub(super) fn folder_drop_target(
    row: gpui::Stateful<gpui::Div>,
    on_drop: impl Fn(&DraggedSession, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    row.w_full()
        .can_drop(|value, _, _| {
            value
                .downcast_ref::<DraggedSession>()
                .is_some_and(|drag| drag.app_session_id > 0)
        })
        .drag_over::<DraggedSession>(|style, _, _, _| {
            style
                .bg(theme().colors.highlight)
                .border_color(theme().colors.indicator)
        })
        .on_drop(move |drag: &DraggedSession, window, cx| {
            cx.stop_propagation();
            on_drop(drag, window, cx);
        })
}

#[cfg(test)]
#[path = "folders_tests.rs"]
mod tests;
