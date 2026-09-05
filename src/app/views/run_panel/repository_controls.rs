use super::{
    super::super::FarcasterApp,
    repository_presentation::{git_identity, repository_sync_metadata},
};
use crate::{
    app::{
        RunPanelView,
        ui::{
            assets::AppIcon,
            primitives::{ButtonTone, dropdown_button, icon_button, section_heading},
            theme::{MONO_FONT_FAMILY, THEME},
        },
    },
    repository::{
        BackendPreference, RepositoryBackend, RepositoryKind, RepositorySyncAction,
        SnapshotIdentity, WorkingCopySnapshot,
    },
};
use gpui::{
    AnyElement, IntoElement, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

pub(super) fn repository_header(
    app: &FarcasterApp,
    snapshot: Option<&WorkingCopySnapshot>,
    entity: WeakEntity<FarcasterApp>,
    panel: WeakEntity<RunPanelView>,
    filtering: bool,
) -> AnyElement {
    let refresh = entity.clone();
    let enabled = app.repository.execution_allowed;
    let syncing = app.repository.sync.action;
    let count = snapshot.map_or(0, |snapshot| {
        snapshot
            .changes
            .iter()
            .map(|change| &change.relative_path)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    });
    let menu = repository_actions(app, snapshot, entity, panel, filtering);

    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(THEME.space.xs)
        .child(
            div()
                .flex()
                .items_center()
                .gap(THEME.space.xs)
                .child(section_heading("Changes"))
                .child(
                    div()
                        .flex_1()
                        .text_size(THEME.type_scale.caption)
                        .text_color(THEME.colors.muted)
                        .child(format!("{count} files")),
                )
                .when(enabled, |row| {
                    row.child(icon_button(
                        "refresh-working-copy",
                        AppIcon::ArrowsClockwise,
                        "Refresh working copy",
                        ButtonTone::Quiet,
                        move |_, cx| {
                            let _ =
                                refresh.update(cx, |this, cx| this.request_repository_refresh(cx));
                        },
                    ))
                })
                .child(menu),
        )
        .child(working_copy_totals(
            app.repository.additions,
            app.repository.deletions,
        ))
        .when_some(snapshot, |section, snapshot| {
            section.child(
                div()
                    .min_w_0()
                    .text_ellipsis()
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.muted)
                    .child(format!(
                        "{} · {} · {}",
                        match snapshot.location.kind {
                            RepositoryKind::Git => "Git",
                            RepositoryKind::Jujutsu => "JJ",
                        },
                        repository_identity_label(snapshot),
                        repository_sync_metadata(&snapshot.identity)
                    )),
            )
        })
        .when_some(syncing, |section, action| {
            section.child(
                div()
                    .text_size(THEME.type_scale.caption)
                    .text_color(THEME.colors.accent)
                    .child(match action {
                        RepositorySyncAction::PullOrFetch => "Syncing repository…",
                        RepositorySyncAction::Push => "Pushing repository…",
                    }),
            )
        })
        .into_any_element()
}

fn repository_actions(
    app: &FarcasterApp,
    snapshot: Option<&WorkingCopySnapshot>,
    entity: WeakEntity<FarcasterApp>,
    panel: WeakEntity<RunPanelView>,
    filtering: bool,
) -> impl IntoElement {
    let project = app.repository.project.clone();
    let enabled = app.repository.execution_allowed;
    let syncing = app.repository.sync.action;
    let identity = snapshot.map(|snapshot| snapshot.identity.clone());
    let kind = snapshot.map(|snapshot| snapshot.location.kind);
    let (git, jj) = RepositoryBackend::available_backends();
    let active = selected_backend(kind, app.repository.preference, git, jj);
    dropdown_button("repository-actions", "Actions", ButtonTone::Quiet, true).dropdown_menu(
        move |mut menu, _, _| {
            for (label, open) in [
                ("Expand all folders", true),
                ("Collapse all folders", false),
            ] {
                let panel = panel.clone();
                let project = project.clone();
                menu = menu.item(PopupMenuItem::new(label).disabled(filtering).on_click(
                    move |_, _, cx| {
                        let _ = panel.update(cx, |view, cx| {
                            view.changes.set_all(&project, open);
                            cx.notify();
                        });
                    },
                ));
            }
            menu = menu.separator();
            for action in [
                RepositorySyncAction::PullOrFetch,
                RepositorySyncAction::Push,
            ] {
                let entity = entity.clone();
                let label = match (kind, action) {
                    (Some(RepositoryKind::Jujutsu), RepositorySyncAction::PullOrFetch) => {
                        "Fetch repository"
                    }
                    (_, RepositorySyncAction::PullOrFetch) => "Pull repository",
                    (_, RepositorySyncAction::Push) => "Push repository",
                };
                let available = enabled
                    && syncing.is_none()
                    && identity
                        .as_ref()
                        .is_some_and(|identity| action.is_available_for(identity));
                menu = menu.item(PopupMenuItem::new(label).disabled(!available).on_click(
                    move |_, _, cx| {
                        let _ =
                            entity.update(cx, |this, cx| this.request_repository_sync(action, cx));
                    },
                ));
            }
            menu = menu.separator();
            for (label, preference, available, kind) in [
                ("Use Git", BackendPreference::Git, git, RepositoryKind::Git),
                (
                    "Use JJ",
                    BackendPreference::Jujutsu,
                    jj,
                    RepositoryKind::Jujutsu,
                ),
            ] {
                let entity = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(active == Some(kind))
                        .disabled(!enabled || !available)
                        .on_click(move |_, window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.set_repository_backend_preference(preference, window, cx)
                            });
                        }),
                );
            }
            menu
        },
    )
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
