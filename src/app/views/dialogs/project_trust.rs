use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use crate::app::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::primitives::{ButtonTone, button, modal},
    app::ui::theme::{MONO_FONT_FAMILY, theme()},
    projects,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let close = entity.clone();
    let project = app
        .project
        .trust_project
        .as_deref()
        .unwrap_or(app.project.path.as_path());
    let backend = app.project.trust_backend;
    let title = backend.map_or_else(
        || "Farcaster project trust".to_owned(),
        |backend| {
            format!(
                "{} project trust",
                crate::agents::backend_display_name(backend)
            )
        },
    );
    let description = backend
        .and_then(crate::agents::project_trust_description)
        .unwrap_or(projects::TRUST_DESCRIPTION);
    let editable_backend = (backend.is_none()
        && app.project.pending_trust_command.is_none()
        && crate::agents::project_trust_description(app.snapshot.harness).is_some())
    .then_some(app.snapshot.harness)
    .flatten();
    let decision = match backend {
        Some(backend) => crate::agents::saved_project_trust(backend, project),
        None => crate::app::project::trust::saved_decision(project),
    };
    let saved = match decision {
        Ok(Some((path, trusted))) => format!(
            "Saved decision: {} ({})",
            if trusted { "trusted" } else { "untrusted" },
            path.display()
        ),
        Ok(None) => "Saved decision: none".into(),
        Err(error) => format!("Saved decision unavailable: {error}"),
    };
    modal(
        "project-trust",
        title,
        &app.overlays.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = close.update(cx, |this, cx| this.dismiss_project_trust(window, cx));
        },
        |surface| {
            let mut choices = div().flex().flex_col().gap(theme().space.xs);
            for (index, option) in projects::options(project)
                .into_iter()
                .enumerate()
            {
                let choice = option.choice;
                let select = entity.clone();
                choices = choices.child(
                    button(
                        ("project-trust-choice", index),
                        option.label,
                        if index == 0 {
                            ButtonTone::Accent
                        } else {
                            ButtonTone::Neutral
                        },
                        true,
                        move |window, cx| {
                            let _ = select.update(cx, |this, cx| {
                                this.save_project_trust(choice, window, cx);
                            });
                        },
                    )
                    .w_full(),
                );
            }

            surface.w(px(640.0)).max_w_full().child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme().space.md)
                    .p(theme().space.md)
                    .child(
                        div()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(theme().type_scale.body_small)
                            .text_color(theme().colors.accent)
                            .child(project.display().to_string()),
                    )
                    .child(
                        div()
                            .text_color(theme().colors.muted)
                            .line_height(theme().type_scale.line_body)
                            .child(description),
                    )
                    .child(
                        div()
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.subtle)
                            .child(saved),
                    )
                    .when_some(app.project.trust_error.clone(), |content, error| {
                        content.child(
                            div()
                                .text_color(theme().colors.error)
                                .child(format!("Trust decision was not saved: {error}")),
                        )
                    })
                    .child(choices)
                    .when_some(editable_backend, |content, backend| {
                        let label = format!("{} project trust…", crate::agents::backend_display_name(backend));
                        let project = project.to_path_buf();
                        let entity = entity.clone();
                        content.child(button("backend-project-trust", label, ButtonTone::Quiet, true, move |window, cx| {
                            let _ = entity.update(cx, |this, cx| this.open_backend_project_trust(backend, project.clone(), window, cx));
                        }))
                    })
                    .child(
                        div()
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.subtle)
                            .child(
                                if app.project.pending_trust_command.is_some()
                                    || app.project.pending_trust_action.is_some()
                                {
                                    "Choose a decision to continue, or close to cancel."
                                } else if backend.is_some() {
                                    "Restart Farcaster after changing this decision."
                                } else {
                                    "This decision controls Farcaster's repository commands."
                                },
                            ),
                    ),
            )
        },
    )
    .into_any_element()
}
