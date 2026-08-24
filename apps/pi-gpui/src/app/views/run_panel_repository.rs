//! Combined Changes sidebar: repository status filtered by Pi's recorded session activity.

use std::collections::HashSet;

use gpui::{
    Anchor, AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Icon,
    menu::{DropdownMenu as _, PopupMenuItem},
    tooltip::Tooltip,
};

use super::{
    super::super::{
        PiApp,
        repository::RepositoryChangeScope,
        views::run_panel_changes::{session_change_matches, session_change_totals},
    },
    run_panel_repository_presentation::{
        accessible_change_path, bounded_message, change_color, change_kind_label,
        change_status_label, display_change_path, git_identity, git_upstream, group_title,
        jj_identity, middle_truncate, preference_label, repository_row_id,
    },
};
use crate::{
    assets::AppIcon,
    primitives::{
        AppIconSize, ButtonTone, activates_button, app_icon, dropdown_button, icon_button,
    },
    repository::{
        BackendPreference, ChangeKind, ChangeLayer, RepositoryKind, SnapshotIdentity,
        WorkingCopyChange, WorkingCopySnapshot,
    },
    session_changes::ChangeSet,
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
        let totals = session_change_totals(&self.changes.set);
        let controls = repository_control_bar(
            self,
            totals.additions,
            totals.deletions,
            entity.clone(),
            move |cx| {
                let _ = refresh.update(cx, |this, cx| {
                    this.request_repository_refresh(cx);
                });
            },
        );

        div()
            .flex()
            .flex_col()
            .gap(THEME.space.xs)
            .child(controls)
            .when(self.changes.set.incomplete, |section| {
                section.child(repository_notice(
                    "Session record was too large to scan in full; totals and filtering may be incomplete",
                    THEME.colors.warning,
                ))
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
            .when_some(self.repository.watcher_error.as_deref(), |section, error| {
                section.child(repository_notice(
                    &format!(
                        "Automatic refresh is unavailable; use Refresh: {}",
                        bounded_message(error)
                    ),
                    THEME.colors.warning,
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
            .when_some(self.repository.snapshot.as_ref(), |section, snapshot| {
                let changes = repository_scope_changes(
                    snapshot,
                    self.repository.scope,
                    &self.changes.set,
                    &self.repository.project,
                );
                section
                    .child(repository_summary(&changes, self.repository.scope))
                    .child(self.repository_changes(snapshot, &changes, entity.clone()))
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
        changes: &[&WorkingCopyChange],
        entity: WeakEntity<Self>,
    ) -> AnyElement {
        if changes.is_empty() {
            let message = if self.repository.scope == RepositoryChangeScope::Session
                && !snapshot.changes.is_empty()
            {
                "No files touched by this session remain changed in the working copy"
            } else {
                match snapshot.location.kind {
                    RepositoryKind::Git => "Working tree and index are clean",
                    RepositoryKind::Jujutsu => "Current change is empty",
                }
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
            let group = changes
                .iter()
                .copied()
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
        let remaining = changes.len().saturating_sub(visible);
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

fn repository_control_bar(
    app: &PiApp,
    additions: Option<u64>,
    deletions: Option<u64>,
    entity: WeakEntity<PiApp>,
    refresh: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let snapshot = app.repository.snapshot.as_ref();
    div()
        .id("repository-control-bar")
        .role(Role::Group)
        .aria_label("Changes: scope, backend, repository identity, and session totals")
        .border(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
        .rounded(THEME.radius)
        .p(THEME.space.xs)
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .child(repository_scope_selector(app, entity.clone()))
                .child(control_divider())
                .child(repository_selector(app, entity))
                .when(app.repository.execution_allowed, |row| {
                    row.child(icon_button(
                        "refresh-working-copy",
                        AppIcon::ArrowsClockwise,
                        "Refresh working copy",
                        ButtonTone::Quiet,
                        move |_, cx| refresh(cx),
                    ))
                }),
        )
        .child(div().h(THEME.border).w_full().bg(THEME.colors.border))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.sm)
                .when_some(snapshot, |row, snapshot| {
                    row.child(repository_identity(snapshot))
                        .child(control_divider())
                })
                .when(snapshot.is_none(), |row| row.child(div().flex_1()))
                .child(session_totals(additions, deletions)),
        )
        .into_any_element()
}

fn control_divider() -> AnyElement {
    div()
        .w(THEME.border)
        .h(px(20.0))
        .flex_none()
        .bg(THEME.colors.border)
        .into_any_element()
}

fn repository_scope_changes<'a>(
    snapshot: &'a WorkingCopySnapshot,
    scope: RepositoryChangeScope,
    session: &ChangeSet,
    project: &std::path::Path,
) -> Vec<&'a WorkingCopyChange> {
    snapshot
        .changes
        .iter()
        .filter(|change| {
            scope == RepositoryChangeScope::All
                || session_change_matches(change, session, project, &snapshot.location.project_root)
        })
        .collect()
}

fn session_totals(additions: Option<u64>, deletions: Option<u64>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .child(div().text_color(THEME.colors.muted).child("Session"))
        .child(
            div()
                .text_color(THEME.colors.success)
                .child(additions.map_or_else(|| "+—".to_owned(), |count| format!("+{count}"))),
        )
        .child(
            div()
                .px(px(3.0))
                .rounded(px(3.0))
                .bg(THEME.colors.diff_deleted)
                .text_color(THEME.colors.text)
                .child(deletions.map_or_else(|| "-—".to_owned(), |count| format!("-{count}"))),
        )
        .into_any_element()
}

fn repository_scope_selector(app: &PiApp, entity: WeakEntity<PiApp>) -> AnyElement {
    let selected = app.repository.scope;
    let label = if app.repository.snapshot.is_none() {
        "Session"
    } else {
        match selected {
            RepositoryChangeScope::All => "Working",
            RepositoryChangeScope::Session => "Session",
        }
    };
    dropdown_button(
        "repository-change-scope",
        label,
        ButtonTone::Neutral,
        app.repository.snapshot.is_some(),
    )
    .icon(Icon::new(AppIcon::Stack))
    .compact()
    .flex_1()
    .min_w_0()
    .font_family(MONO_FONT_FAMILY)
    .text_color(THEME.colors.text)
    .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
        let mut menu = menu.label("Show changes");
        for (scope, label) in [
            (RepositoryChangeScope::All, "All working copy"),
            (RepositoryChangeScope::Session, "This session"),
        ] {
            let target = entity.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .checked(selected == scope)
                    .on_click(move |_, _, cx| {
                        let _ = target.update(cx, |this, cx| {
                            this.set_repository_change_scope(scope, cx);
                        });
                    }),
            );
        }
        menu
    })
    .into_any_element()
}

fn repository_selector(app: &PiApp, entity: WeakEntity<PiApp>) -> AnyElement {
    let label = preference_label(
        app.repository.preference,
        app.repository
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.location.kind),
    );
    let label = if label == "Auto · Jujutsu" {
        "Auto · Jj"
    } else {
        label.as_str()
    };
    let selected = app.repository.preference;
    dropdown_button(
        "repository-backend",
        label,
        ButtonTone::Neutral,
        app.repository.execution_allowed,
    )
    .icon(Icon::new(AppIcon::GitBranch))
    .compact()
    .flex_1()
    .min_w_0()
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
    let primary_label = format!("{backend} · {primary}");
    let full_label = secondary.as_ref().map_or_else(
        || primary_label.clone(),
        |secondary| format!("{primary_label} · {secondary}"),
    );
    div()
        .id("repository-identity")
        .aria_label(format!("Repository: {full_label}"))
        .tooltip(move |window, cx| Tooltip::new(full_label.clone()).build(window, cx))
        .min_w_0()
        .flex_1()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .child(app_icon(AppIcon::GitBranch, AppIconSize::Inline).text_color(THEME.colors.muted))
        .child(
            div()
                .min_w_0()
                .flex_1()
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
                        .child(primary_label),
                )
                .children(secondary.map(|secondary| {
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child(secondary)
                })),
        )
        .into_any_element()
}

fn repository_summary(changes: &[&WorkingCopyChange], scope: RepositoryChangeScope) -> AnyElement {
    let paths = changes
        .iter()
        .map(|change| change.relative_path.as_path())
        .collect::<HashSet<_>>()
        .len();
    let conflicts = changes
        .iter()
        .filter(|change| change.kind == ChangeKind::Conflict)
        .count();
    let mut summary = format!("{} {}", paths, if paths == 1 { "file" } else { "files" });
    if changes.len() != paths {
        summary.push_str(&format!(" · {} entries", changes.len()));
    }
    if scope == RepositoryChangeScope::Session {
        summary.push_str(" · touched this session");
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
