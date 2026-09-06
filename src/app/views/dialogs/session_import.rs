use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _, px,
};

use crate::app::FarcasterApp;
use crate::{
    agents,
    app::OVERLAY_KEY_CONTEXT,
    app::session::import::import_harnesses,
    app::ui::primitives::{ButtonTone, FeedbackTone, button, feedback, modal},
    app::ui::theme::THEME,
    sessions::SessionSummary,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let dialog = app.session_import.as_ref().expect("visible import");
    let dismiss = entity.clone();
    let harness = dialog.harness.clone();
    let harness_name = agents::backend_display_name(&harness);
    let loading = dialog.loading;
    let error = dialog.error.clone();
    let candidates = dialog.candidates.clone();
    let selected = dialog.selected.clone();
    let selected_count = selected.len();
    modal(
        "import-sessions",
        "Import sessions",
        &dialog.focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_session_import(window, cx));
        },
        |surface| {
            let cancel = entity.clone();
            let confirm = entity.clone();
            surface.w(px(640.0)).max_w_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(THEME.space.md)
                    .p(THEME.space.md)
                    .child(
                        div()
                            .text_size(THEME.type_scale.display)
                            .child("Import sessions"),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.body)
                            .text_color(THEME.colors.muted)
                            .child(
                                "Choose one harness, review what is on disk, then import the sessions you want. Farcaster does not watch session files.",
                            ),
                    )
                    .child(harness_picker(entity.clone(), &harness))
                    .when_some(error, |this, message| {
                        this.child(feedback("import-error", message, FeedbackTone::Error))
                    })
                    .child(if loading {
                        div()
                            .text_color(THEME.colors.muted)
                            .child(format!("Looking for {harness_name} sessions…"))
                    } else if candidates.is_empty() {
                        div().text_color(THEME.colors.muted).child(format!(
                            "No new {harness_name} sessions on disk."
                        ))
                    } else {
                        candidate_list(entity.clone(), &candidates, &selected)
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(THEME.space.sm)
                            .child(button(
                                "cancel-session-import",
                                "Cancel",
                                ButtonTone::Neutral,
                                true,
                                move |window, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.close_session_import(window, cx)
                                    });
                                },
                            ))
                            .child(button(
                                "confirm-session-import",
                                if selected_count == 0 {
                                    "Import".into()
                                } else {
                                    format!("Import {selected_count}")
                                },
                                ButtonTone::Accent,
                                selected_count > 0,
                                move |window, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.confirm_session_import(window, cx)
                                    });
                                },
                            )),
                    ),
            )
        },
    )
    .into_any_element()
}

fn harness_picker(entity: WeakEntity<FarcasterApp>, selected: &str) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .gap(THEME.space.xs)
        .children(import_harnesses().into_iter().map(|harness| {
            let active = harness == selected;
            let entity = entity.clone();
            let id = format!("import-harness-{harness}");
            let label = agents::backend_display_name(&harness);
            button(
                id,
                label,
                if active {
                    ButtonTone::Accent
                } else {
                    ButtonTone::Neutral
                },
                true,
                move |_, cx| {
                    let harness = harness.clone();
                    let _ = entity.update(cx, |this, cx| {
                        this.select_session_import_harness(harness, cx);
                    });
                },
            )
        }))
}

fn candidate_list(
    entity: WeakEntity<FarcasterApp>,
    candidates: &[SessionSummary],
    selected: &std::collections::HashSet<std::path::PathBuf>,
) -> gpui::Div {
    let all = entity.clone();
    let none = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .child(
            div()
                .flex()
                .gap(THEME.space.sm)
                .child(button(
                    "import-select-all",
                    "Select all",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        let _ = all.update(cx, |this, cx| {
                            this.set_session_import_selection(true, cx);
                        });
                    },
                ))
                .child(button(
                    "import-select-none",
                    "Select none",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        let _ = none.update(cx, |this, cx| {
                            this.set_session_import_selection(false, cx);
                        });
                    },
                )),
        )
        .child(
            div()
                .id("import-session-list")
                .max_h(px(320.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(THEME.space.xs)
                .children(candidates.iter().map(|session| {
                    let path = session.path.clone();
                    let checked = selected.contains(&path);
                    let entity = entity.clone();
                    let title = if session.parent_session.is_some() {
                        format!("↳ {}", session.title)
                    } else {
                        session.title.clone()
                    };
                    let project = session
                        .project
                        .file_name()
                        .and_then(|name| name.to_str())
                        .filter(|name| !name.is_empty())
                        .map_or_else(|| session.project.display().to_string(), str::to_owned);
                    button(
                        format!("import-session-{}", session.path.display()),
                        format!("{title}  ·  {project}"),
                        if checked {
                            ButtonTone::Accent
                        } else {
                            ButtonTone::Neutral
                        },
                        true,
                        move |_, cx| {
                            let path = path.clone();
                            let _ = entity.update(cx, |this, cx| {
                                this.toggle_session_import_candidate(path, cx);
                            });
                        },
                    )
                })),
        )
}
