use super::{
    FarcasterApp,
    colors::{ColorTarget, color_menu, palette_color},
    drag::DraggedSession,
    groups::{ActiveSessionItem, SessionRailKind},
    rendering::session_section_header,
};
use crate::app::{
    PickerScope, ProjectPickerIntent,
    session_folders::SessionFolders,
    ui::{
        assets::AppIcon,
        primitives::{AppIconSize, ButtonTone, ContextMenuTrigger, app_icon, button},
        theme::theme,
    },
};
use gpui::{
    Anchor, AnyElement, Entity, FontWeight, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Styled as _, WeakEntity, div, px,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};

#[derive(Clone)]
pub(super) struct FolderHeader {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) color: u8,
    pub(super) collapsed: bool,
    pub(super) sessions: usize,
}

#[derive(Clone)]
pub(super) enum FolderRow {
    Session(Box<ActiveSessionItem>),
    Header(Box<FolderHeader>),
    New,
}

impl FolderRow {
    pub(super) fn session(&self) -> Option<&ActiveSessionItem> {
        match self {
            Self::Session(item) => Some(item),
            _ => None,
        }
    }
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
            sessions: members.len(),
        })));
        if !folder.collapsed {
            rows.extend(
                members
                    .into_iter()
                    .map(|item| FolderRow::Session(Box::new(item))),
            );
        }
    }
    rows.push(FolderRow::New);
    rows
}

pub(super) fn folder_header(
    folder: Option<Box<FolderHeader>>,
    editing: bool,
    input: Entity<InputState>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let Some(folder) = folder else {
        return new_folder_row(editing, input, entity);
    };
    let FolderHeader {
        id,
        name,
        color,
        collapsed,
        sessions,
    } = *folder;
    let drop_entity = entity.clone();
    let edit_entity = entity.clone();
    let menu_entity = entity.clone();
    let new_entity = entity.clone();
    let color_entity = entity.clone();
    let context_entity = entity.clone();
    let toggle_entity = entity.clone();
    let cancel_entity = entity;
    let section = div().w_full().flex().flex_col();
    let mut row = session_section_header()
        .id(format!("session-folder-{id}"))
        .group("session-folder-header")
        .w_full()
        .gap(theme().space.xs);
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


    row = folder_drop_target(row, move |drag, _, cx| {
        let _ = drop_entity.update(cx, |this, cx| {
            this.assign_session_folder(drag.app_session_id, Some(id), cx);
            this.clear_session_drop_target(cx);
        });
    });
    row = row
        .child(
            folder_action(Button::new(format!("folder-toggle-{id}")).ghost())
                .accessibility_label(if collapsed {
                    "Open folder"
                } else {
                    "Minimise folder"
                })
                .tooltip(if collapsed {
                    "Open folder"
                } else {
                    "Minimise folder"
                })
                .text_color(theme().colors.muted)
                .child(app_icon(
                    if collapsed {
                        AppIcon::CaretRight
                    } else {
                        AppIcon::CaretDown
                    },
                    AppIconSize::Inline,
                ))
                .on_click(move |_, _, cx| {
                    let _ = toggle_entity.update(cx, |this, cx| {
                        this.set_folder_collapsed(id, !collapsed, cx);
                    });
                }),
        )
        .child(
            div()
                .flex_none()
                .text_color(palette_color(color))
                .child(app_icon(AppIcon::Folder, AppIconSize::Inline)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name),
        )
        .child(
            div()
                .flex_none()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.subtle)
                .child(if collapsed {
                    sessions.to_string()
                } else {
                    String::new()
                }),
        );

    row = row.child(
        folder_action(
            crate::app::ui::primitives::dropdown_button(
                format!("folder-menu-{id}"),
                "⋯",
                ButtonTone::Quiet,
                true,
            )
            .dropdown_caret(false)
            .px(px(0.0)),
        )
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, window, cx| {
            let rename = menu_entity.clone();
            let delete = menu_entity.clone();
            let color_entity = color_entity.clone();
            menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                let _ = rename.update(cx, |this, cx| this.begin_folder_edit(Some(id), window, cx));
            }))
            .submenu("Colour", window, cx, move |menu, _, _| {
                color_menu(
                    menu,
                    Some(color),
                    color_entity.clone(),
                    ColorTarget::Folder(id),
                )
            })
            .item(
                PopupMenuItem::new("Delete folder").on_click(move |_, _, cx| {
                    let _ = delete.update(cx, |this, cx| {
                        let mut next = this.sessions.folders.clone();
                        next.remove(id);
                        this.save_session_folders(next, cx);
                    });
                }),
            )
        }),
    );
    row = row.child(
        folder_action(Button::new(format!("new-session-in-folder-{id}")).ghost())
            .px(px(0.0))
            .accessibility_label("New session in folder")
            .tooltip("New session in folder")
            .text_color(theme().colors.muted)
            .child(app_icon(AppIcon::Plus, AppIconSize::Inline))
            .on_click(move |_, window, cx| {
                let _ = new_entity.update(cx, |this, cx| {
                    this.open_picker(
                        PickerScope::Projects(ProjectPickerIntent::NewSessionInFolder(id)),
                        window,
                        cx,
                    );
                });
            }),
    );
    let header = ContextMenuTrigger::new(format!("folder-context-{id}"), row.into_any_element())
        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, window, cx| {
            let rename = context_entity.clone();
            let colour = context_entity.clone();
            menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                let _ = rename.update(cx, |this, cx| this.begin_folder_edit(Some(id), window, cx));
            }))
            .submenu("Colour", window, cx, move |menu, _, _| {
                color_menu(menu, Some(color), colour.clone(), ColorTarget::Folder(id))
            })
        });
    section.child(header).into_any_element()
}

fn new_folder_row(
    editing: bool,
    input: Entity<InputState>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let drop_entity = entity.clone();
    let edit_entity = entity.clone();
    let cancel_entity = entity.clone();
    let section = div().w_full().flex().flex_col().pt(theme().space.sm);
    let mut row = session_section_header()
        .id("session-folder-none")
        .group("session-folder-header")
        .w_full()
        .gap(theme().space.xs)
        .h(theme().size(40.0));
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
                    "Create",
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
            this.begin_folder_edit(None, window, cx);
            if let Some(edit) = &mut this.sessions.editing_folder {
                edit.session = Some(drag.app_session_id);
            }
            this.clear_session_drop_target(cx);
        });
    });
    section
        .child(
            row.hover(|row| row.bg(theme().colors.highlight))
                .child(new_folder_button(move |window, cx| {
                    let _ =
                        edit_entity.update(cx, |this, cx| this.begin_folder_edit(None, window, cx));
                })),
        )
        .into_any_element()
}

fn folder_action(button: Button) -> Button {
    button
        .size(theme().controls.icon_button)
        .cursor_pointer()
        .opacity(0.0)
        .group_hover("session-folder-header", |style| style.opacity(1.0))
        .focus(|style| style.opacity(1.0))
}

fn new_folder_button(on_press: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static) -> Button {
    Button::new("new-session-folder")
        .text()
        .accessibility_label("New folder")
        .w_full()
        .h_full()
        .px(px(0.0))
        .cursor_pointer()
        .text_color(theme().colors.muted)
        .group_hover("session-folder-header", |button| {
            button.text_color(theme().colors.text)
        })
        .child(
            div()
                .w_full()
                .text_size(theme().type_scale.body_small)
                .font_weight(FontWeight::NORMAL)
                .child("+ New folder"),
        )
        .on_click(move |_, window, cx| on_press(window, cx))
}

pub(super) fn folder_drop_target(
    row: gpui::Stateful<gpui::Div>,
    on_drop: impl Fn(&DraggedSession, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    row.w_full()
        .can_drop(|value, _, _| {
            value.downcast_ref::<DraggedSession>().is_some_and(|drag| {
                drag.app_session_id > 0 && drag.kind == SessionRailKind::Project
            })
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
