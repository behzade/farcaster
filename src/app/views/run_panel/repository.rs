use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::tooltip::Tooltip;

#[cfg(test)]
use super::repository_controls::selected_backend;
use super::{
    super::super::FarcasterApp,
    repository_controls::{file_action, repository_header},
    repository_presentation::{
        accessible_change_path, bounded_message, change_color, change_kind_label,
        change_status_label, display_change_path, file_path_labels, group_title, repository_row_id,
    },
};
#[cfg(test)]
use crate::repository::BackendPreference;
use crate::{
    app::ui::theme::THEME,
    app::ui::{
        assets::AppIcon,
        file_icons::file_icon,
        primitives::{AppIconSize, activates_button, app_icon},
    },
    repository::{RepositoryEdit, RepositoryKind, WorkingCopyChange, WorkingCopySnapshot},
};

use super::{
    RepositoryView,
    change_tree::{self, TreeRow},
};
use crate::app::RunPanelView;
use gpui_component::input::Input;

impl FarcasterApp {
    pub(super) fn render_repository(
        &self,
        entity: WeakEntity<Self>,
        panel: WeakEntity<RunPanelView>,
        browser: &RepositoryView<'_>,
    ) -> AnyElement {
        let snapshot = self.repository.snapshot.as_ref();
        let header = repository_header(
            self,
            snapshot,
            entity.clone(),
            panel.clone(),
            !browser.query.trim().is_empty(),
        );

        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(header)
            .when(
                self.repository.loading && !self.repository.initialized,
                |section| {
                    section.child(
                        div()
                            .id("repository-loading")
                            .role(Role::Status)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.accent)
                            .child("Reading working copy…"),
                    )
                },
            )
            .when_some(
                self.repository.preference_error.as_deref(),
                |section, error| {
                    section.child(repository_notice(
                        &format!("Backend choice was not saved: {}", bounded_message(error)),
                        THEME.colors.warning,
                    ))
                },
            )
            .when_some(
                self.repository.watcher_error.as_deref(),
                |section, error| {
                    section.child(repository_error_notice(
                        "Auto-refresh unavailable. Use Refresh to try again.",
                        error,
                        THEME.colors.warning,
                    ))
                },
            )
            .when_some(self.repository.sync.error.as_deref(), |section, error| {
                section.child(repository_notice(
                    &format!("Repository sync failed: {}", bounded_message(error)),
                    THEME.colors.error,
                ))
            })
            .when_some(self.repository.error.as_deref(), |section, error| {
                let message = if self.repository.snapshot.is_some() {
                    "Could not refresh changes. Showing the previous result."
                } else {
                    "Could not read this repository. Check the project folder and refresh."
                };
                section.child(repository_error_notice(message, error, THEME.colors.error))
            })
            .when_some(snapshot, |section, snapshot| {
                section
                    .child(
                        Input::new(browser.search)
                            .bg(gpui::rgba(0))
                            .border_color(gpui::rgba(0))
                            .aria_label("Filter changed files")
                            .prefix(app_icon(AppIcon::MagnifyingGlass, AppIconSize::Inline)),
                    )
                    .child(self.repository_changes(
                        snapshot,
                        entity.clone(),
                        panel.clone(),
                        browser,
                    ))
            })
            .when(!self.repository.execution_allowed, |section| {
                section.child(
                    div()
                        .id("repository-disabled")
                        .role(Role::Status)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.warning)
                        .child("Repository integration is disabled for this untrusted project"),
                )
            })
            .when(
                self.repository.execution_allowed
                    && self.repository.snapshot.is_none()
                    && self.repository.error.is_none()
                    && self.repository.initialized,
                |section| {
                    section.child(
                        div()
                            .id("repository-not-found")
                            .role(Role::Status)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child("No repository found for this project"),
                    )
                },
            )
            .into_any_element()
    }

    fn repository_changes(
        &self,
        snapshot: &WorkingCopySnapshot,
        entity: WeakEntity<Self>,
        panel: WeakEntity<RunPanelView>,
        browser: &RepositoryView<'_>,
    ) -> AnyElement {
        let rows = change_tree::rows(
            snapshot.changes.iter().enumerate().map(|(index, change)| {
                (
                    index,
                    change.relative_path.as_path(),
                    change.original_relative_path.as_deref(),
                    change.counts,
                )
            }),
            browser.query,
            &self.repository.project,
            browser.state,
        );
        div()
            .id("repository-files")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(browser.scroll)
            .children(rows.iter().filter_map(|row| {
                match row {
                    TreeRow::Folder {
                        path,
                        label,
                        count,
                        counts,
                        depth,
                        open,
                    } => {
                        let project = self.repository.project.clone();
                        let path = path.clone();
                        let panel = panel.clone();
                        let accessible = format!(
                            "{} {}",
                            if *open { "Collapse" } else { "Expand" },
                            path.display()
                        );
                        Some(
                            crate::app::ui::primitives::tree_folder_row(
                                format!("repository-folder-{}", path.display()),
                                label.clone(),
                                *depth,
                                *open,
                                browser.query.trim().is_empty(),
                                move |_, cx| {
                                    let _ = panel.update(cx, |view, cx| {
                                        view.changes.toggle(&project, &path);
                                        cx.notify();
                                    });
                                },
                            )
                            .aria_label(accessible)
                            .when(!*open, |row| {
                                row.child(crate::app::ui::primitives::folder_change_summary(
                                    *count, *counts,
                                ))
                            })
                            .into_any_element(),
                        )
                    }
                    TreeRow::File { index, depth } => self
                        .repository_change_row(&snapshot.changes[*index], entity.clone())
                        .map(|row| {
                            div()
                                .pl(px(*depth as f32 * 12.0))
                                .child(row)
                                .into_any_element()
                        }),
                }
            }))
            .when(rows.is_empty(), |list| {
                list.child(repository_notice(
                    if !browser.query.trim().is_empty() {
                        "No matching files"
                    } else {
                        match snapshot.location.kind {
                            RepositoryKind::Git => "Working tree and index are clean",
                            RepositoryKind::Jujutsu => "Current change is empty",
                        }
                    },
                    THEME.colors.subtle,
                ))
            })
            .into_any_element()
    }

    fn repository_change_row(
        &self,
        change: &WorkingCopyChange,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let focus = self.repository.row_focus.get(&change.target.key)?.clone();
        let click_entity = entity.clone();
        let click_path = change.target.absolute_path();
        let key_entity = entity.clone();
        let key_path = change.target.absolute_path();
        let row_id = repository_row_id(&change.target.key);
        let action_group: gpui::SharedString = format!("repository-actions-{row_id}").into();
        let selecting = !self.repository.edits.selection.paths.is_empty();
        let selected = self
            .repository
            .edits
            .selection
            .paths
            .contains(&change.relative_path);
        let select_entity = entity.clone();
        let select_path = change.relative_path.clone();
        let discard_path = change.relative_path.clone();
        let editable = self.repository.execution_allowed
            && self.repository.sync.action.is_none()
            && self.repository.edits.pending.is_none()
            && change.kind != crate::repository::ChangeKind::Conflict;
        let full_path = format!("{} · ⌥ Open diff", display_change_path(change));
        let (filename, _) = file_path_labels(&change.relative_path);
        let status = change_status_label(change).to_owned();
        let layer = group_title(change.layer);
        let state = change_kind_label(&change.kind);
        let accessible_path = accessible_change_path(change);
        let accessible = format!("Edit {layer} {state} file {accessible_path} in Neovim");
        let target = div()
            .id(("repository-change", row_id))
            .group(action_group.clone())
            .track_focus(&focus)
            .role(Role::Button)
            .aria_label(accessible)
            .tooltip(move |window, cx| Tooltip::new(full_path.clone()).build(window, cx))
            .tab_index(0)
            .on_mouse_down(
                gpui::MouseButton::Left,
                crate::app::ui::primitives::preserve_pointer_focus,
            )
            .min_w_0()
            .w_full()
            .h(px(24.0))
            .px(THEME.space.xs)
            .rounded(THEME.radius)
            .flex()
            .items_center()
            .gap(THEME.space.xs)
            .hover(|row| row.bg(THEME.colors.hover))
            .when(selected, |row| row.bg(THEME.colors.selection))
            .focus(|row| row.bg(THEME.colors.selection))
            .cursor_pointer()
            .on_click(move |event, window, cx| {
                let _ = click_entity.update(cx, |this, cx| {
                    this.open_file_editor_with_diff(
                        click_path.clone(),
                        None,
                        event.modifiers().alt,
                        window,
                        cx,
                    );
                });
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.open_file_editor(key_path.clone(), window, cx);
                    });
                }
            })
            .child(
                file_action(
                    ("select-repository-file", row_id),
                    format!(
                        "{} {} for commit",
                        if selected { "Deselect" } else { "Select" },
                        change.relative_path.display()
                    ),
                    move |_, cx| {
                        if editable {
                            let _ = select_entity.update(cx, |this, cx| {
                                this.toggle_repository_file(select_path.clone(), cx)
                            });
                        }
                    },
                )
                .role(Role::CheckBox)
                .aria_toggled(if selected {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .when(!selecting, |control| {
                    control
                        .opacity(0.0)
                        .group_hover(action_group.clone(), |control| control.opacity(1.0))
                        .focus_visible(|control| control.opacity(1.0))
                })
                .child(
                    div()
                        .size(px(14.0))
                        .border(THEME.border)
                        .border_color(THEME.colors.muted)
                        .when(!editable, |checkbox| checkbox.opacity(0.4))
                        .rounded(px(2.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |checkbox| {
                            checkbox.bg(THEME.colors.accent).child(
                                app_icon(AppIcon::Check, AppIconSize::Inline)
                                    .text_color(THEME.colors.surface),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(THEME.space.xs)
                    .child(file_icon(&change.relative_path))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.text)
                            .text_ellipsis()
                            .child(filename),
                    )
                    .when(
                        change.layer == crate::repository::ChangeLayer::GitIndex,
                        |label| {
                            label.child(
                                div()
                                    .text_size(THEME.type_scale.caption)
                                    .text_color(THEME.colors.subtle)
                                    .child("Staged"),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .w(px(14.0))
                    .flex_none()
                    .text_size(THEME.type_scale.caption)
                    .text_color(if change.kind == crate::repository::ChangeKind::Modified {
                        THEME.colors.subtle
                    } else {
                        change_color(&change.kind)
                    })
                    .child(status),
            )
            .child(
                div()
                    .w(px(20.0))
                    .flex_none()
                    .when(!selecting && editable, |slot| {
                        slot.child(
                            file_action(
                                ("discard-repository-file", row_id),
                                if matches!(
                                    change.kind,
                                    crate::repository::ChangeKind::Added
                                        | crate::repository::ChangeKind::Untracked
                                ) {
                                    "Delete file…"
                                } else {
                                    "Discard file changes…"
                                },
                                move |window, cx| {
                                    let _ = entity.update(cx, |this, cx| {
                                        this.review_repository_edit(
                                            RepositoryEdit::Discard,
                                            Some(discard_path.clone()),
                                            window,
                                            cx,
                                        )
                                    });
                                },
                            )
                            .opacity(0.0)
                            .group_hover(action_group, |control| control.opacity(1.0))
                            .focus_visible(|control| control.opacity(1.0))
                            .child(app_icon(
                                AppIcon::ArrowCounterClockwise,
                                AppIconSize::Inline,
                            )),
                        )
                    }),
            );
        Some(target.into_any_element())
    }
}

fn repository_error_notice(message: &str, detail: &str, color: gpui::Rgba) -> AnyElement {
    let detail = bounded_message(detail);
    repository_notice(message, color)
        .min_w_0()
        .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
        .into_any_element()
}

fn repository_notice(message: &str, color: gpui::Rgba) -> gpui::Stateful<gpui::Div> {
    div()
        .id(message.to_owned())
        .role(Role::Status)
        .text_size(THEME.type_scale.caption)
        .text_color(color)
        .child(message.to_owned())
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
