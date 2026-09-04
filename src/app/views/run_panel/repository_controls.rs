use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use super::{
    super::super::FarcasterApp,
    repository_presentation::{git_identity, repository_sync_metadata},
};
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, ButtonTone, activates_button, app_icon, icon_button},
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    repository::{
        BackendPreference, RepositoryBackend, RepositoryKind, RepositorySyncAction,
        SnapshotIdentity, WorkingCopySnapshot,
    },
};

pub(super) fn repository_header(
    app: &FarcasterApp,
    snapshot: Option<&WorkingCopySnapshot>,
    entity: WeakEntity<FarcasterApp>,
    refresh: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let (git_available, jj_available) = RepositoryBackend::available_backends();
    let active = selected_backend(
        snapshot.map(|snapshot| snapshot.location.kind),
        app.repository.preference,
        git_available,
        jj_available,
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
            git_available,
            jj_available,
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

pub(super) fn repository_sync_row(
    app: &FarcasterApp,
    snapshot: &WorkingCopySnapshot,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let metadata = repository_sync_metadata(&snapshot.identity);
    let syncing = app.repository.sync.action;
    let actions_enabled = app.repository.execution_allowed && syncing.is_none();
    let pull_enabled =
        actions_enabled && RepositorySyncAction::PullOrFetch.is_available_for(&snapshot.identity);
    let push_enabled =
        actions_enabled && RepositorySyncAction::Push.is_available_for(&snapshot.identity);
    let pull_label = match snapshot.location.kind {
        RepositoryKind::Git => format!("Pull {metadata}"),
        RepositoryKind::Jujutsu => "Fetch repository".to_owned(),
    };
    let push_label = match &snapshot.identity {
        SnapshotIdentity::Git(_) => format!("Push {metadata}"),
        SnapshotIdentity::Jujutsu(identity) => identity.bookmarks.first().map_or_else(
            || "Push unavailable: current change has no bookmark".to_owned(),
            |bookmark| format!("Push bookmark {bookmark}"),
        ),
    };
    let pull = entity.clone();
    let push = entity;

    div()
        .id("repository-sync-row")
        .role(Role::Group)
        .aria_label("Repository remote synchronization")
        .min_w_0()
        .flex()
        .items_center()
        .gap(THEME.space.xs)
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(THEME.colors.subtle)
                .child(metadata),
        )
        .when_some(syncing, |row, action| {
            let label = match (snapshot.location.kind, action) {
                (RepositoryKind::Git, RepositorySyncAction::PullOrFetch) => "Pulling repository",
                (RepositoryKind::Jujutsu, RepositorySyncAction::PullOrFetch) => {
                    "Fetching repository"
                }
                (_, RepositorySyncAction::Push) => "Pushing repository",
            };
            row.child(
                div()
                    .id("repository-syncing-status")
                    .role(Role::Status)
                    .aria_label(label)
                    .size(THEME.controls.icon_button)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(THEME.colors.accent)
                    .child(app_icon(AppIcon::SpinnerGap, AppIconSize::Control)),
            )
        })
        .when(pull_enabled, |row| {
            row.child(icon_button(
                "repository-pull-or-fetch",
                AppIcon::ArrowDown,
                pull_label,
                ButtonTone::Quiet,
                move |_, cx| {
                    let _ = pull.update(cx, |this, cx| {
                        this.request_repository_sync(RepositorySyncAction::PullOrFetch, cx);
                    });
                },
            ))
        })
        .when(push_enabled, |row| {
            row.child(icon_button(
                "repository-push",
                AppIcon::ArrowUp,
                push_label,
                ButtonTone::Quiet,
                move |_, cx| {
                    let _ = push.update(cx, |this, cx| {
                        this.request_repository_sync(RepositorySyncAction::Push, cx);
                    });
                },
            ))
        })
        .into_any_element()
}

pub(super) fn selected_backend(
    discovered: Option<RepositoryKind>,
    preference: BackendPreference,
    git_available: bool,
    jj_available: bool,
) -> Option<RepositoryKind> {
    discovered.or(match preference {
        BackendPreference::Git if git_available => Some(RepositoryKind::Git),
        BackendPreference::Jujutsu if jj_available => Some(RepositoryKind::Jujutsu),
        BackendPreference::Auto | BackendPreference::Git | BackendPreference::Jujutsu => None,
    })
}

fn backend_toggle(
    active: Option<RepositoryKind>,
    enabled: bool,
    git_available: bool,
    jj_available: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(3.0))
        .when(jj_available, |toggle| {
            toggle.child(backend_option(
                "repository-backend-jj",
                "JJ",
                BackendPreference::Jujutsu,
                active == Some(RepositoryKind::Jujutsu),
                enabled,
                entity.clone(),
            ))
        })
        .when(jj_available && git_available, |toggle| {
            toggle.child(div().text_color(THEME.colors.subtle).child("/"))
        })
        .when(git_available, |toggle| {
            toggle.child(backend_option(
                "repository-backend-git",
                "Git",
                BackendPreference::Git,
                active == Some(RepositoryKind::Git),
                enabled,
                entity,
            ))
        })
        .into_any_element()
}

fn backend_option(
    id: &'static str,
    label: &'static str,
    preference: BackendPreference,
    active: bool,
    enabled: bool,
    entity: WeakEntity<FarcasterApp>,
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
        .on_click(move |_, window, cx| {
            if enabled {
                let _ = entity.update(cx, |this, cx| {
                    this.set_repository_backend_preference(preference, window, cx);
                });
            }
        })
        .on_key_down(move |event, window, cx| {
            if enabled && activates_button(event) {
                cx.stop_propagation();
                let _ = key_entity.update(cx, |this, cx| {
                    this.set_repository_backend_preference(preference, window, cx);
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
