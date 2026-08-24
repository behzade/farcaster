//! Authoritative working-copy status, separate from Pi's recorded session activity.

use std::collections::HashSet;

use gpui::{
    Anchor, AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{
    super::super::PiApp,
    run_panel_repository_presentation::{
        accessible_change_path, bounded_message, change_color, change_kind_label,
        change_status_label, display_change_path, git_identity, git_upstream, group_title,
        jj_identity, middle_truncate, preference_label, repository_row_id,
    },
};
use crate::{
    assets::AppIcon,
    primitives::{ButtonTone, activates_button, dropdown_button, icon_button, section_heading},
    repository::{
        BackendPreference, ChangeKind, ChangeLayer, RepositoryKind, SnapshotIdentity,
        WorkingCopyChange, WorkingCopySnapshot,
    },
    theme::{MONO_FONT_FAMILY, THEME},
};

const MAX_VISIBLE_CHANGES: usize = 10;
const GROUP_ORDER: [ChangeLayer; 5] = [
    ChangeLayer::GitConflict,
    ChangeLayer::GitIndex,
    ChangeLayer::GitWorkingTree,
    ChangeLayer::GitUntracked,
    ChangeLayer::JujutsuWorkingCopy,
];

impl PiApp {
    pub(super) fn render_repository(&self, entity: WeakEntity<Self>) -> AnyElement {
        let refresh = entity.clone();
        let heading = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(THEME.space.xs)
            .child(section_heading("Working copy"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(repository_selector(self, entity.clone()))
                    .when(self.repository.execution_allowed, |controls| {
                        controls.child(icon_button(
                            "refresh-working-copy",
                            AppIcon::ArrowsClockwise,
                            "Refresh working copy",
                            ButtonTone::Quiet,
                            move |_, cx| {
                                let _ = refresh.update(cx, |this, cx| {
                                    this.request_repository_refresh(cx);
                                });
                            },
                        ))
                    }),
            );

        div()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(heading)
            .when(self.repository.loading, |section| {
                section.child(
                    div()
                        .id("repository-loading")
                        .role(Role::Status)
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.accent)
                        .child(if self.repository.snapshot.is_some() {
                            "Refreshing working copy…"
                        } else {
                            "Reading working copy…"
                        }),
                )
            })
            .when_some(
                self.repository.preference_error.as_deref(),
                |section, error| {
                    section.child(repository_notice(
                        &format!("Backend choice was not saved: {}", bounded_message(error)),
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
            .when_some(self.repository.snapshot.as_ref(), |section, snapshot| {
                section
                    .child(repository_identity(snapshot))
                    .child(repository_summary(snapshot))
                    .child(self.repository_changes(snapshot, entity.clone()))
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
                    && !self.repository.loading,
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
            let message = match snapshot.location.kind {
                RepositoryKind::Git => "Working tree and index are clean",
                RepositoryKind::Jujutsu => "Current change is empty",
            };
            return div()
                .id("repository-clean")
                .role(Role::Status)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.success)
                .child(message)
                .into_any_element();
        }

        let mut visible = 0_usize;
        let mut body = div().flex().flex_col().gap(px(2.0));
        for layer in GROUP_ORDER {
            let group = snapshot
                .changes
                .iter()
                .filter(|change| change.layer == layer)
                .collect::<Vec<_>>();
            if group.is_empty() {
                continue;
            }
            let group_count = group.len();
            body = body.child(
                div()
                    .pt(THEME.space.xs)
                    .text_size(THEME.type_scale.caption)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(THEME.colors.subtle)
                    .child(format!("{} ({group_count})", group_title(layer))),
            );
            for change in group.into_iter().take(MAX_VISIBLE_CHANGES - visible) {
                visible = visible.saturating_add(1);
                if let Some(row) = self.repository_change_row(change, entity.clone()) {
                    body = body.child(row);
                }
            }
        }
        let remaining = snapshot.changes.len().saturating_sub(visible);
        body.when(remaining > 0, |body| {
            body.child(
                div()
                    .px(THEME.space.xs)
                    .pt(THEME.space.xs)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.subtle)
                    .child(format!("{remaining} more changes")),
            )
        })
        .into_any_element()
    }

    fn repository_change_row(
        &self,
        change: &WorkingCopyChange,
        entity: WeakEntity<Self>,
    ) -> Option<AnyElement> {
        let backend = self.repository.backend.clone()?;
        let focus = self.repository.row_focus.get(&change.target.key)?.clone();
        let click_backend = backend.clone();
        let click_focus = focus.clone();
        let click_target = change.target.clone();
        let click_entity = entity.clone();
        let key_focus = focus.clone();
        let key_target = change.target.clone();
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
                let backend = click_backend.clone();
                let target = click_target.clone();
                let focus = click_focus.clone();
                let _ = click_entity.update(cx, |this, cx| {
                    this.open_repository_diff(backend, target, focus, window, cx);
                });
            })
            .on_key_down(move |event, window, cx| {
                if activates_button(event) {
                    cx.stop_propagation();
                    let backend = backend.clone();
                    let target = key_target.clone();
                    let focus = key_focus.clone();
                    let _ = key_entity.update(cx, |this, cx| {
                        this.open_repository_diff(backend, target, focus, window, cx);
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

fn repository_selector(app: &PiApp, entity: WeakEntity<PiApp>) -> AnyElement {
    let label = preference_label(
        app.repository.preference,
        app.repository
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.location.kind),
    );
    let selected = app.repository.preference;
    dropdown_button(
        "repository-backend",
        label,
        ButtonTone::Quiet,
        app.repository.execution_allowed,
    )
    .font_family(MONO_FONT_FAMILY)
    .text_color(THEME.colors.text)
    .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
        let mut menu = menu.label("Working copy backend");
        for (preference, label) in [
            (BackendPreference::Auto, "Auto"),
            (BackendPreference::Jujutsu, "Jujutsu"),
            (BackendPreference::Git, "Git"),
        ] {
            let target = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .checked(selected == preference)
                    .on_click(move |_, _, cx| {
                        let _ = target.update(cx, |this, cx| {
                            this.set_repository_backend_preference(preference, cx);
                        });
                    }),
            );
        }
        menu
    })
    .into_any_element()
}

fn repository_identity(snapshot: &WorkingCopySnapshot) -> AnyElement {
    let (backend, primary, secondary) = match &snapshot.identity {
        SnapshotIdentity::Git(identity) => ("Git", git_identity(identity), git_upstream(identity)),
        SnapshotIdentity::Jujutsu(identity) => (
            "Jujutsu",
            jj_identity(identity),
            (!identity.description.is_empty()).then(|| identity.description.clone()),
        ),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.body_small)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(THEME.colors.text)
                .child(format!("{backend} · {primary}")),
        )
        .children(secondary.map(|secondary| {
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.subtle)
                .child(secondary)
        }))
        .into_any_element()
}

fn repository_summary(snapshot: &WorkingCopySnapshot) -> AnyElement {
    let paths = snapshot
        .changes
        .iter()
        .map(|change| change.relative_path.as_path())
        .collect::<HashSet<_>>()
        .len();
    let conflicts = snapshot
        .changes
        .iter()
        .filter(|change| change.kind == ChangeKind::Conflict)
        .count();
    let mut summary = format!("{} {}", paths, if paths == 1 { "file" } else { "files" });
    if snapshot.changes.len() != paths {
        summary.push_str(&format!(" · {} entries", snapshot.changes.len()));
    }
    if conflicts > 0 {
        summary.push_str(&format!(
            " · {conflicts} conflict{}",
            if conflicts == 1 { "" } else { "s" }
        ));
    }
    div()
        .id("repository-summary")
        .role(Role::Status)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(if conflicts > 0 {
            THEME.colors.warning
        } else {
            THEME.colors.subtle
        })
        .child(summary)
        .into_any_element()
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
