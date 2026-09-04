use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

#[cfg(test)]
use super::repository_controls::selected_backend;
use super::{
    super::super::FarcasterApp,
    repository_controls::{repository_header, repository_sync_row},
    repository_presentation::{
        accessible_change_path, bounded_message, change_color, change_kind_label,
        change_status_label, display_change_path, group_title, middle_truncate, repository_row_id,
    },
};
#[cfg(test)]
use crate::repository::BackendPreference;
use crate::{
    app::ui::primitives::{activates_button, disclosure_button},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    repository::{RepositoryKind, WorkingCopyChange, WorkingCopySnapshot},
};

const INITIAL_VISIBLE_CHANGES: usize = 5;
const EXPAND_CHANGE_PAGE: usize = 20;

impl FarcasterApp {
    pub(super) fn render_repository(&self, entity: WeakEntity<Self>) -> AnyElement {
        let refresh = entity.clone();
        let snapshot = self.repository.snapshot.as_ref();
        let header = repository_header(self, snapshot, entity.clone(), move |cx| {
            let _ = refresh.update(cx, |this, cx| this.request_repository_refresh(cx));
        });

        div()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(header)
            .when_some(snapshot, |section, snapshot| {
                section.child(repository_sync_row(self, snapshot, entity.clone()))
            })
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
                section.child(self.repository_changes(snapshot, entity.clone()))
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
    ) -> AnyElement {
        if snapshot.changes.is_empty() {
            return div()
                .id("repository-clean")
                .role(Role::Status)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(match snapshot.location.kind {
                    RepositoryKind::Git => "Working tree and index are clean",
                    RepositoryKind::Jujutsu => "Current change is empty",
                })
                .into_any_element();
        }

        let (visible, remaining, expand_count) =
            change_page(snapshot.changes.len(), self.repository.visible_changes);
        let expand = entity.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(
                snapshot
                    .changes
                    .iter()
                    .take(visible)
                    .filter_map(|change| self.repository_change_row(change, entity.clone())),
            )
            .when(remaining > 0, |changes| {
                changes.child(
                    div()
                        .pt(THEME.space.xs)
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.subtle)
                        .child(format!("{remaining} more files"))
                        .child(disclosure_button(
                            "expand-repository-changes",
                            false,
                            format!("Show {expand_count} more files"),
                            move |_, cx| {
                                let _ = expand.update(cx, |this, cx| {
                                    this.expand_repository_changes(cx);
                                });
                            },
                        )),
                )
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
        let display_path = middle_truncate(&full_path, 42);
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
            .tab_index(0)
            .min_w_0()
            .flex_1()
            .px(THEME.space.xs)
            .py(px(6.0))
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
                    .font_family(MONO_FONT_FAMILY)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(change_color(&change.kind))
                    .child(status),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.caption)
                    .child(display_path),
            );
        Some(target.into_any_element())
    }
}

fn change_page(total: usize, requested: usize) -> (usize, usize, usize) {
    let visible = requested.max(INITIAL_VISIBLE_CHANGES).min(total);
    let remaining = total.saturating_sub(visible);
    (visible, remaining, remaining.min(EXPAND_CHANGE_PAGE))
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

    #[test]
    fn repository_files_start_at_five_and_expand_twenty_at_a_time() {
        assert_eq!(change_page(50, 5), (5, 45, 20));
        assert_eq!(change_page(50, 25), (25, 25, 20));
        assert_eq!(change_page(30, 25), (25, 5, 5));
        assert_eq!(change_page(3, 5), (3, 0, 0));
    }
}
