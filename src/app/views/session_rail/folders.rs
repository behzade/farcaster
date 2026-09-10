use super::{
    FarcasterApp,
    drag::DraggedSession,
    groups::{ActiveSessionItem, SessionRailKind},
    rendering::session_section_header,
};
use crate::app::{
    PickerScope, ProjectPickerIntent,
    session_folders::SessionFolders,
    ui::{
        assets::AppIcon,
        primitives::{AppIconSize, ButtonTone, app_icon, button},
        theme::THEME,
    },
};
use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, WeakEntity, div, px,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

#[derive(Clone)]
pub(super) enum FolderRow {
    Session(ActiveSessionItem),
    Header(u64, String),
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
            .entry(folders.folder_for(item.app_session_id()))
            .or_default()
            .push(item);
    }
    let mut rows = sections
        .remove(&None)
        .unwrap_or_default()
        .into_iter()
        .map(FolderRow::Session)
        .collect::<Vec<_>>();
    for folder in &folders.folders {
        rows.push(FolderRow::Header(folder.id, folder.name.clone()));
        rows.extend(
            sections
                .remove(&Some(folder.id))
                .unwrap_or_default()
                .into_iter()
                .map(FolderRow::Session),
        );
    }
    rows.push(FolderRow::New);
    rows
}

pub(super) fn folder_header(
    id: Option<u64>,
    name: String,
    editing: bool,
    input: gpui::Entity<gpui_component::input::InputState>,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let drop_entity = entity.clone();
    let edit_entity = entity.clone();
    let menu_entity = entity.clone();
    let new_entity = entity.clone();
    let cancel_entity = entity;
    let mut section = div().w_full().flex().flex_col().pt(THEME.space.sm);
    if id.is_some() {
        section = section.child(
            div()
                .mx(THEME.space.md)
                .mb(THEME.space.sm)
                .h(THEME.border)
                .flex_none()
                .bg(THEME.colors.border),
        );
    }
    let mut row = session_section_header()
        .id(format!("session-folder-{id:?}"))
        .group("session-folder-header")
        .w_full()
        .gap(THEME.space.xs);
    if id.is_none() || editing {
        row = row.h(px(40.0));
    }
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
                    if id.is_some() { "Save" } else { "Create" },
                    ButtonTone::Neutral,
                    true,
                    move |_, cx| {
                        let _ = commit.update(cx, |this, cx| this.commit_folder_edit(cx));
                    },
                )),
            )
            .into_any_element();
    }
    let Some(id) = id else {
        row = folder_drop_target(row, move |drag, window, cx| {
            let _ = drop_entity.update(cx, |this, cx| {
                this.begin_folder_edit(None, window, cx);
                if let Some(edit) = &mut this.editing_folder {
                    edit.session = Some(drag.app_session_id);
                }
                this.clear_session_drop_target(cx);
            });
        });
        return section
            .child(
                row.hover(|row| row.bg(THEME.colors.hover))
                    .child(new_folder_button(move |window, cx| {
                        let _ = edit_entity
                            .update(cx, |this, cx| this.begin_folder_edit(None, window, cx));
                    })),
            )
            .into_any_element();
    };

    row = folder_drop_target(row, move |drag, _, cx| {
        let _ = drop_entity.update(cx, |this, cx| {
            this.assign_session_folder(drag.app_session_id, Some(id), cx);
            this.clear_session_drop_target(cx);
        });
    });
    row = row.child(
        div()
            .flex_1()
            .min_w_0()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(name),
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
            .px(px(4.0)),
        )
        .dropdown_menu(move |menu, _, _| {
            let rename = menu_entity.clone();
            let delete = menu_entity.clone();
            menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                let _ = rename.update(cx, |this, cx| this.begin_folder_edit(Some(id), window, cx));
            }))
            .item(
                PopupMenuItem::new("Delete folder").on_click(move |_, _, cx| {
                    let _ = delete.update(cx, |this, cx| {
                        let mut next = this.session_folders.clone();
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
            .text_color(THEME.colors.muted)
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
    section.child(row).into_any_element()
}

fn folder_action(button: Button) -> Button {
    button
        .size(px(24.0))
        .cursor_pointer()
        .opacity(0.0)
        .group_hover("session-folder-header", |style| style.opacity(1.0))
        .focus(|style| style.opacity(1.0))
}

fn new_folder_button(on_press: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static) -> Button {
    Button::new("new-session-folder")
        .ghost()
        .accessibility_label("New folder")
        .w_full()
        .h_full()
        .px(px(0.0))
        .cursor_pointer()
        .text_color(THEME.colors.muted)
        .group_hover("session-folder-header", |button| {
            button.text_color(THEME.colors.text)
        })
        .child(
            div()
                .w_full()
                .text_size(THEME.type_scale.body_small)
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
                .bg(THEME.colors.hover)
                .border_color(THEME.colors.accent)
        })
        .on_drop(move |drag: &DraggedSession, window, cx| {
            cx.stop_propagation();
            on_drop(drag, window, cx);
        })
}

#[cfg(test)]
#[path = "folders_tests.rs"]
mod tests;
