use std::path::PathBuf;

use super::{
    FarcasterApp, ReorderPosition,
    colors::{ColorTarget, color_menu, palette_color},
    drag::DraggedSession,
    group_drag::{GroupTarget, draggable_title, group_drop_target},
    groups::{ActiveSessionItem, SessionRailKind},
    rendering::session_section_header,
};
use crate::app::{
    session_folders::SessionFolders,
    ui::{
        assets::AppIcon,
        primitives::{
            AppIconSize, AppTooltip as _, ButtonTone, ContextMenuTrigger, activates_button,
            app_icon, button, icon_control,
        },
        theme::theme,
    },
};
use gpui::{
    Anchor, AnyElement, Entity, InteractiveElement as _, IntoElement as _, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
};

#[derive(Clone)]
pub(super) struct FolderHeader {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) color: Option<u8>,
    pub(super) collapsed: bool,
    pub(super) count: usize,
}

#[derive(Clone)]
pub(super) enum FolderRow {
    Session(Box<ActiveSessionItem>),
    Header(Box<FolderHeader>),
    Project {
        path: PathBuf,
        label: String,
        collapsed: bool,
        count: usize,
    },
    New,
}

pub(super) fn folder_rows(
    items: Vec<ActiveSessionItem>,
    folders: &SessionFolders,
) -> Vec<FolderRow> {
    let mut sections = std::collections::BTreeMap::<Option<u64>, Vec<ActiveSessionItem>>::new();
    for item in items {
        sections
            .entry(folders.folder_for(item.app_session_id()))
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
            count: members.len(),
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

pub(super) fn project_group_rows(
    items: Vec<ActiveSessionItem>,
    collapsed: &std::collections::HashSet<PathBuf>,
    order: &[PathBuf],
) -> Vec<FolderRow> {
    let mut projects = std::collections::BTreeMap::<PathBuf, Vec<ActiveSessionItem>>::new();
    for item in items {
        projects
            .entry(item.project().to_path_buf())
            .or_default()
            .push(item);
    }
    let labels = project_labels(projects.keys().cloned().collect());
    let mut groups = projects.into_iter().zip(labels).collect::<Vec<_>>();
    let positions = order
        .iter()
        .enumerate()
        .map(|(index, path)| (path, index))
        .collect::<std::collections::HashMap<_, _>>();
    groups.sort_by_key(|((path, _), _)| positions.get(path).copied().unwrap_or(usize::MAX));
    let mut rows = Vec::new();
    for ((path, items), label) in groups {
        let collapsed = collapsed.contains(&path);
        rows.push(FolderRow::Project {
            path,
            label,
            collapsed,
            count: items.len(),
        });
        if !collapsed {
            rows.extend(
                items
                    .into_iter()
                    .map(|item| FolderRow::Session(Box::new(item))),
            );
        }
    }
    rows
}

fn project_labels(paths: Vec<PathBuf>) -> Vec<String> {
    let suffixes = paths
        .iter()
        .map(|path| {
            let components = path.components().collect::<Vec<_>>();
            (0..components.len())
                .rev()
                .map(|start| components[start..].iter().collect::<PathBuf>())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut counts = std::collections::HashMap::<&PathBuf, usize>::new();
    for suffix in suffixes.iter().flatten() {
        *counts.entry(suffix).or_default() += 1;
    }
    suffixes
        .iter()
        .zip(&paths)
        .map(|(suffixes, path)| {
            suffixes
                .iter()
                .find(|suffix| counts[suffix] == 1)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect()
}

fn group_title(
    id: String,
    label: String,
    collapsed: bool,
    count: usize,
    on_press: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let on_press = std::rc::Rc::new(on_press);
    let click = on_press.clone();
    div()
        .id(id)
        .debug_selector(|| "session-group-title".into())
        .role(gpui::Role::Button)
        .aria_label(format!("{label}, {count} chats"))
        .aria_expanded(!collapsed)
        .tab_index(0)
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .cursor_pointer()
        .focus_visible(|title| {
            title
                .border(theme().border)
                .border_color(theme().colors.indicator)
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            click(window, cx);
        })
        .on_key_down(move |event, window, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(window, cx);
            }
        })
        .child(
            div()
                .min_w_0()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label),
        )
        .when(collapsed, |title| {
            title.child(
                div()
                    .debug_selector(|| "session-group-count".into())
                    .flex_none()
                    .text_color(theme().colors.subtle)
                    .child(format!("({count})")),
            )
        })
}

pub(super) fn project_header(
    project: PathBuf,
    label: String,
    collapsed: bool,
    count: usize,
    color: Option<u8>,
    drop_position: Option<ReorderPosition>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let drop_entity = entity.clone();
    let drag_entity = entity.clone();
    let target = GroupTarget::Project(project.clone());
    let drag_target = target.clone();
    let drag_label = label.clone();
    let menu_entity = entity.clone();
    let menu_project = project.clone();
    let toggle = entity.clone();
    let toggle_project = project.clone();
    let row = session_section_header()
        .id(format!("session-project-{}", project.display()))
        .debug_selector({
            let project = project.clone();
            move || format!("session-project-{}", project.display())
        })
        .group("session-project-header")
        .w_full()
        .px(theme().space.sm)
        .child(
            draggable_title(
                group_title(
                    format!("project-label-{}", project.display()),
                    label,
                    collapsed,
                    count,
                    move |_, cx| {
                        let _ = toggle.update(cx, |this, cx| {
                            if !this.sessions.collapsed_projects.remove(&toggle_project) {
                                this.sessions
                                    .collapsed_projects
                                    .insert(toggle_project.clone());
                            }
                            this.notify_session_rail(cx);
                        });
                    },
                ),
                drag_target,
                drag_label,
                drag_entity,
            )
            .text_color(color.map_or(theme().colors.muted, palette_color))
            .app_tooltip(project.display().to_string()),
        )
        .child(
            group_menu_button(
                format!("project-menu-{}", project.display()),
                "session-project-header",
            )
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                let entity = menu_entity.clone();
                let project = menu_project.clone();
                menu.submenu("Color", window, cx, move |menu, _, _| {
                    color_menu(
                        menu,
                        color,
                        entity.clone(),
                        ColorTarget::Project(project.clone()),
                    )
                })
            }),
        )
        .child(
            icon_control(
                format!("new-session-in-project-{}", project.display()),
                "New session in project",
            )
            .child(app_icon(AppIcon::Plus, AppIconSize::Inline))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                let _ = entity.update(cx, |this, cx| this.new_session(project.clone(), window, cx));
            }),
        );
    group_drop_target(row, target, drop_position, drop_entity).into_any_element()
}

pub(super) fn new_folder_row(
    editing: bool,
    input: Entity<InputState>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let edit = entity.clone();
    let cancel = entity.clone();
    let row = session_section_header().id("new-folder-row").w_full();
    if editing {
        return row
            .on_action(move |_: &gpui_component::input::Escape, _, cx| {
                cx.stop_propagation();
                let _ = cancel.update(cx, |this, cx| this.cancel_session_title_edit(cx));
            })
            .child(Input::new(&input).flex_1().min_w_0().appearance(true))
            .child(button(
                "create-folder",
                "Create",
                ButtonTone::Neutral,
                true,
                move |_, cx| {
                    let _ = edit.update(cx, |this, cx| this.commit_folder_edit(cx));
                },
            ))
            .into_any_element();
    }
    folder_drop_target(row, move |drag, window, cx| {
        let _ = entity.update(cx, |this, cx| {
            this.begin_folder_edit(None, window, cx);
            if let Some(edit) = &mut this.sessions.editing_folder {
                edit.session = Some(drag.app_session_id);
            }
            this.clear_session_drop_target(cx);
        });
    })
    .child(button(
        "new-folder",
        "+ New folder",
        ButtonTone::Quiet,
        true,
        move |window, cx| {
            let _ = edit.update(cx, |this, cx| this.begin_folder_edit(None, window, cx));
        },
    ))
    .into_any_element()
}

pub(super) fn folder_header(
    folder: FolderHeader,
    editing: bool,
    input: Entity<InputState>,
    drop_position: Option<ReorderPosition>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let drag_entity = entity.clone();
    let reorder_entity = entity.clone();
    let FolderHeader {
        id,
        name,
        color,
        collapsed,
        count,
    } = folder;
    let toggle = entity.clone();
    let drop_entity = entity.clone();
    let edit_entity = entity.clone();
    let new_entity = entity.clone();
    let context_entity = entity.clone();
    let hover_entity = entity.clone();
    let cancel_entity = entity;
    let section = div().w_full().flex().flex_col();
    let mut row = session_section_header()
        .id(format!("session-folder-{id}"))
        .debug_selector(move || format!("session-folder-{id}"))
        .group("session-folder-header")
        .w_full()
        .px(theme().space.sm)
        .cursor_pointer()
        .hover(|row| row.bg(theme().colors.highlight))
        .on_hover(move |hovered: &bool, _, cx| {
            if *hovered {
                let _ = hover_entity.update(cx, |this, cx| this.prefetch_folder(id, cx));
            }
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
    row = group_drop_target(row, GroupTarget::Folder(id), drop_position, reorder_entity);
    let drag_label = name.clone();
    row = row.child(
        draggable_title(
            group_title(
                format!("folder-label-{id}"),
                name,
                collapsed,
                count,
                move |_, cx| {
                    let _ = toggle.update(cx, |this, cx| {
                        let mut next = this.sessions.folders.clone();
                        if next.set_collapsed(id, !collapsed) {
                            this.save_session_folders(next, cx);
                        }
                    });
                },
            ),
            GroupTarget::Folder(id),
            drag_label,
            drag_entity,
        )
        .text_color(color.map_or(theme().colors.muted, palette_color)),
    );

    let folder_menu = move |menu, window: &mut gpui::Window, cx: &mut gpui::Context<PopupMenu>| {
        custom_folder_menu(menu, id, color, context_entity.clone(), window, cx)
    };
    row = row.child(
        group_menu_button(format!("folder-menu-{id}"), "session-folder-header")
            .dropdown_menu_with_anchor(Anchor::TopRight, folder_menu.clone()),
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
                        this.open_picker(
                            crate::app::PickerScope::Projects(
                                crate::app::ProjectPickerIntent::NewSessionInFolder(id),
                            ),
                            window,
                            cx,
                        );
                    });
                }),
            ),
    );
    let header = ContextMenuTrigger::new(format!("folder-context-{id}"), row.into_any_element())
        .dropdown_menu_with_anchor(Anchor::TopLeft, folder_menu)
        .mouse_button(MouseButton::Right);
    section.child(header).into_any_element()
}

fn group_menu_button(id: String, group: &'static str) -> ContextMenuTrigger {
    ContextMenuTrigger::new(
        id.clone(),
        icon_control(format!("{id}-button"), "Group options")
            .debug_selector(|| "session-group-menu".into())
            .opacity(0.0)
            .group_hover(group, |button| button.opacity(1.0))
            .focus(|button| button.opacity(1.0))
            .child("⋯")
            .into_any_element(),
    )
    .w(theme().controls.icon_button)
    .flex_none()
}

fn custom_folder_menu(
    menu: PopupMenu,
    id: u64,
    color: Option<u8>,
    entity: WeakEntity<FarcasterApp>,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let rename = entity.clone();
    let colour = entity.clone();
    let delete = entity.clone();
    menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
        let _ = rename.update(cx, |this, cx| this.begin_folder_edit(Some(id), window, cx));
    }))
    .submenu("Color", window, cx, move |menu, _, _| {
        color_menu(menu, color, colour.clone(), ColorTarget::Folder(id))
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
    .item(
        PopupMenuItem::new("Delete folder and chats…").on_click(move |_, window, cx| {
            let _ = entity.update(cx, |this, cx| this.request_folder_delete(id, window, cx));
        }),
    )
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
