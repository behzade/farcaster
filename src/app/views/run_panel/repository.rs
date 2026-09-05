use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::tooltip::Tooltip;

#[cfg(test)]
use super::repository_controls::selected_backend;
use super::{
    super::super::FarcasterApp,
    repository_controls::repository_header,
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
    repository::{RepositoryKind, WorkingCopyChange, WorkingCopySnapshot},
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
                    section.child(repository_notice(
                        &format!(
                            "Automatic refresh is unavailable; use Refresh: {}",
                            bounded_message(error)
                        ),
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
                    format!(
                        "Refresh failed; showing the previous result: {}",
                        bounded_message(error)
                    )
                } else {
                    bounded_message(error)
                };
                section.child(repository_notice(&message, THEME.colors.error))
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
                        count: _,
                        depth,
                        open,
                    } => {
                        let project = self.repository.project.clone();
                        let path = path.clone();
                        let panel = panel.clone();
                        let key_panel = panel.clone();
                        let key_project = project.clone();
                        let key_path = path.clone();
                        let filtering = !browser.query.trim().is_empty();
                        Some(
                            div()
                                .id(format!("repository-folder-{}", path.display()))
                                .role(Role::Button)
                                .aria_label(format!(
                                    "{} {}",
                                    if *open { "Collapse" } else { "Expand" },
                                    path.display()
                                ))
                                .aria_expanded(*open)
                                .w_full()
                                .min_w_0()
                                .h(px(24.0))
                                .pl(px(*depth as f32 * 12.0 + 4.0))
                                .pr(THEME.space.xs)
                                .flex()
                                .items_center()
                                .gap(THEME.space.xs)
                                .rounded(THEME.radius)
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .when(!filtering, |row| {
                                    row.tab_index(0)
                                        .cursor_pointer()
                                        .hover(|row| row.bg(THEME.colors.hover))
                                        .focus_visible(|row| row.bg(THEME.colors.selection))
                                        .on_click(move |_, _, cx| {
                                            let _ = panel.update(cx, |view, cx| {
                                                view.changes.toggle(&project, &path);
                                                cx.notify();
                                            });
                                        })
                                        .on_key_down(move |event, _, cx| {
                                            if activates_button(event) {
                                                cx.stop_propagation();
                                                let _ = key_panel.update(cx, |view, cx| {
                                                    view.changes.toggle(&key_project, &key_path);
                                                    cx.notify();
                                                });
                                            }
                                        })
                                })
                                .child(div().w(px(14.0)).flex_none().child(app_icon(
                                    if *open {
                                        AppIcon::CaretDown
                                    } else {
                                        AppIcon::CaretRight
                                    },
                                    AppIconSize::Inline,
                                )))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .text_ellipsis()
                                        .child(label.clone()),
                                )
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
        let key_entity = entity;
        let key_path = change.target.absolute_path();
        let row_id = repository_row_id(&change.target.key);
        let full_path = display_change_path(change);
        let (filename, _) = file_path_labels(&change.relative_path);
        let status = change_status_label(change).to_owned();
        let layer = group_title(change.layer);
        let state = change_kind_label(&change.kind);
        let accessible_path = accessible_change_path(change);
        let accessible = format!("Edit {layer} {state} file {accessible_path} in Neovim");
        let target = div()
            .id(("repository-change", row_id))
            .track_focus(&focus)
            .role(Role::Button)
            .aria_label(accessible)
            .tooltip(move |window, cx| Tooltip::new(full_path.clone()).build(window, cx))
            .tab_index(0)
            .min_w_0()
            .w_full()
            .h(px(24.0))
            .px(THEME.space.xs)
            .rounded(THEME.radius)
            .flex()
            .items_center()
            .gap(THEME.space.xs)
            .hover(|row| row.bg(THEME.colors.hover))
            .focus(|row| row.bg(THEME.colors.selection))
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let _ = click_entity.update(cx, |this, cx| {
                    this.open_file_editor(click_path.clone(), window, cx);
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
            );
        Some(target.into_any_element())
    }
}

fn repository_notice(message: &str, color: gpui::Rgba) -> AnyElement {
    div()
        .id(message.to_owned())
        .role(Role::Status)
        .text_size(THEME.type_scale.caption)
        .text_color(color)
        .child(message.to_owned())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_does_not_select_jj_before_discovery() {
        assert_eq!(
            selected_backend(None, BackendPreference::Auto, true, true),
            None
        );
        assert_eq!(
            selected_backend(None, BackendPreference::Jujutsu, true, false),
            None
        );
        assert_eq!(
            selected_backend(
                Some(RepositoryKind::Git),
                BackendPreference::Auto,
                true,
                true,
            ),
            Some(RepositoryKind::Git)
        );
    }
}
