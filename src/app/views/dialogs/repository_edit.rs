use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::Textarea;

use crate::{
    app::{
        FarcasterApp, OVERLAY_KEY_CONTEXT,
        ui::{
            primitives::{ButtonTone, button, modal, submit_textarea},
            theme::{MONO_FONT_FAMILY, THEME},
        },
    },
    repository::RepositoryEdit,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let pending = app
        .repository
        .edits
        .pending
        .as_ref()
        .expect("visible repository review");
    let commit = pending.action == RepositoryEdit::Commit;
    let deletes = pending
        .review
        .as_ref()
        .is_some_and(|review| review.removes_file());
    let title = if commit {
        match pending.paths.len() {
            1 => "Commit · 1 file".to_owned(),
            count => format!("Commit · {count} files"),
        }
    } else if deletes {
        "Delete file?".to_owned()
    } else {
        "Discard changes?".to_owned()
    };
    let label = if pending.applying {
        if commit {
            "Committing…"
        } else {
            "Discarding…"
        }
    } else if commit {
        "Commit"
    } else if deletes {
        "Delete file"
    } else {
        "Discard changes"
    };
    let dismiss = entity.clone();
    let cancel = entity.clone();
    modal(
        "repository-edit",
        title.clone(),
        &pending.focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_repository_edit(window, cx));
        },
        |surface| {
            surface.w(px(480.0)).child(
                div()
                    .p(THEME.space.md)
                    .flex()
                    .flex_col()
                    .gap(THEME.space.sm)
                    .child(div().text_size(THEME.type_scale.body).child(title))
                    .when(!commit && !deletes, |body| {
                        body.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.muted)
                                .child("Includes staged and unstaged changes."),
                        )
                    })
                    .child(
                        div()
                            .id("repository-review-files")
                            .max_h(px(120.0))
                            .overflow_y_scroll()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.muted)
                            .children(
                                pending
                                    .paths
                                    .iter()
                                    .map(|path| div().child(path.display().to_string())),
                            ),
                    )
                    .when(commit, |body| {
                        body.child(submit_textarea(Textarea::new(&pending.input)))
                    })
                    .when(pending.preparing(), |body| {
                        body.child(div().child("Checking selected files…"))
                    })
                    .when_some(pending.error.as_ref(), |body, error| {
                        body.child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .text_color(THEME.colors.error)
                                .child(error.clone()),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-repository-edit",
                                "Cancel",
                                ButtonTone::Neutral,
                                !pending.applying,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_repository_edit(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "confirm-repository-edit",
                                label,
                                if commit {
                                    ButtonTone::Accent
                                } else {
                                    ButtonTone::Danger
                                },
                                pending.can_apply(cx),
                                move |window, cx| {
                                    let _ = entity.update(cx, |this, cx| {
                                        this.confirm_repository_edit(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}
