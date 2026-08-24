//! Flat working-copy header and paged repository file list.

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::{
    super::super::PiApp,
    run_panel_repository_presentation::{
        accessible_change_path, bounded_message, change_color, change_kind_label,
        change_status_label, display_change_path, git_identity, group_title, middle_truncate,
        repository_row_id,
    },
};
use crate::{
    assets::AppIcon,
    primitives::{ButtonTone, activates_button, disclosure_button, icon_button},
    repository::{
        BackendPreference, RepositoryKind, SnapshotIdentity, WorkingCopyChange, WorkingCopySnapshot,
    },
    theme::{MONO_FONT_FAMILY, THEME},
};

const INITIAL_VISIBLE_CHANGES: usize = 5;
const EXPAND_CHANGE_PAGE: usize = 20;

impl PiApp {
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
            .when(
                self.repository.snapshot.is_none()
                    && (self.repository.initialized || !self.repository.loading),
                |section| section.child(self.render_session_change_fallback(entity.clone())),
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
        let click_focus = focus.clone();
        let click_key = change.target.key.clone();
        let click_entity = entity.clone();
        let key_focus = focus.clone();
        let key = change.target.key.clone();
        let key_entity = entity.clone();
        let editor_entity = entity;
        let editor_path = change.target.absolute_path();
        let row_id = repository_row_id(&change.target.key);
        let full_path = display_change_path(change);
        let display_path = middle_truncate(&full_path, 42);
        let status = change_status_label(change).to_owned();
        let layer = group_title(change.layer);
        let state = change_kind_label(&change.kind);
        let accessible_path = accessible_change_path(change);
        let accessible = format!("Open {layer} diff for {state} file {accessible_path}");
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
                let key = click_key.clone();
                let focus = click_focus.clone();
                let _ = click_entity.update(cx, |this, cx| {
                    this.open_current_repository_diff(key, focus, window, cx);
                });
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    let key = key.clone();
                    let focus = key_focus.clone();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.open_current_repository_diff(key, focus, window, cx);
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
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(2.0))
                .child(target)
                .when(change.target.exists, |row| {
                    row.child(icon_button(
                        ("edit-repository-change", row_id),
                        AppIcon::PencilSimple,
                        "Edit in Neovim",
                        ButtonTone::Quiet,
                        move |window, cx| {
                            let _ = editor_entity.update(cx, |this, cx| {
                                this.open_file_editor(editor_path.clone(), window, cx);
                            });
                        },
                    ))
                })
                .into_any_element(),
        )
    }
}

fn repository_header(
    app: &PiApp,
    snapshot: Option<&WorkingCopySnapshot>,
    entity: WeakEntity<PiApp>,
    refresh: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let active = snapshot.map(|snapshot| snapshot.location.kind).unwrap_or(
        match app.repository.preference {
            BackendPreference::Git => RepositoryKind::Git,
            BackendPreference::Auto | BackendPreference::Jujutsu => RepositoryKind::Jujutsu,
        },
    );
    let identity = snapshot.map(repository_identity_label);
    let dirty = snapshot.is_some_and(|snapshot| !snapshot.changes.is_empty());
    div()
        .id("repository-control-bar")
        .role(Role::Group)
        .aria_label("Working copy backend, totals, and identity")
        .min_w_0()
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.body_small)
        .child(backend_toggle(
            active,
            app.repository.execution_allowed,
            entity,
        ))
        .child(working_copy_totals(
            app.repository.additions,
            app.repository.deletions,
        ))
        .when_some(identity, |row, identity| {
            row.child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.text)
                    .child(format!("{identity}{}", if dirty { "*" } else { "" })),
            )
        })
        .child(div().min_w_0().flex_1())
        .when(app.repository.execution_allowed, |row| {
            row.child(icon_button(
                "refresh-working-copy",
                AppIcon::ArrowsClockwise,
                "Refresh working copy",
                ButtonTone::Quiet,
                move |_, cx| refresh(cx),
            ))
        })
        .into_any_element()
}

fn backend_toggle(active: RepositoryKind, enabled: bool, entity: WeakEntity<PiApp>) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(3.0))
        .child(backend_option(
            "repository-backend-jj",
            "JJ",
            BackendPreference::Jujutsu,
            active == RepositoryKind::Jujutsu,
            enabled,
            entity.clone(),
        ))
        .child(div().text_color(THEME.colors.subtle).child("/"))
        .child(backend_option(
            "repository-backend-git",
            "Git",
            BackendPreference::Git,
            active == RepositoryKind::Git,
            enabled,
            entity,
        ))
        .into_any_element()
}

fn backend_option(
    id: &'static str,
    label: &'static str,
    preference: BackendPreference,
    active: bool,
    enabled: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let key_entity = entity.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(format!(
            "{} {label} working copy backend",
            if active { "Selected" } else { "Use" }
        ))
        .tab_index(if enabled { 0 } else { -1 })
        .cursor_pointer()
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            THEME.colors.text
        } else {
            THEME.colors.subtle
        })
        .hover(|option| option.text_color(THEME.colors.accent))
        .on_click(move |_, _, cx| {
            if enabled {
                let _ = entity.update(cx, |this, cx| {
                    this.set_repository_backend_preference(preference, cx);
                });
            }
        })
        .on_key_down(move |event, _, cx| {
            if enabled && activates_button(event) {
                cx.stop_propagation();
                let _ = key_entity.update(cx, |this, cx| {
                    this.set_repository_backend_preference(preference, cx);
                });
            }
        })
        .child(label)
        .into_any_element()
}

fn working_copy_totals(additions: Option<u64>, deletions: Option<u64>) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .child(
            div()
                .text_color(THEME.colors.success)
                .child(additions.map_or_else(|| "+—".to_owned(), |count| format!("+{count}"))),
        )
        .child(
            div()
                .text_color(THEME.colors.error)
                .child(deletions.map_or_else(|| "-—".to_owned(), |count| format!("-{count}"))),
        )
        .into_any_element()
}

fn repository_identity_label(snapshot: &WorkingCopySnapshot) -> String {
    match &snapshot.identity {
        SnapshotIdentity::Git(identity) => git_identity(identity),
        SnapshotIdentity::Jujutsu(identity) => identity.change_id.chars().take(8).collect(),
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
    fn repository_files_start_at_five_and_expand_twenty_at_a_time() {
        assert_eq!(change_page(50, 5), (5, 45, 20));
        assert_eq!(change_page(50, 25), (25, 25, 20));
        assert_eq!(change_page(30, 25), (25, 5, 5));
        assert_eq!(change_page(3, 5), (3, 0, 0));
    }
}
