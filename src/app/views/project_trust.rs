use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use super::super::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    primitives::{ButtonTone, button, modal},
    project_trust,
    theme::{MONO_FONT_FAMILY, THEME},
};

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let close = entity.clone();
    let project = app
        .project_trust_project
        .as_deref()
        .unwrap_or(app.project.as_path());
    let saved = match project_trust::saved_decision(project) {
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
        "Project trust",
        &app.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = close.update(cx, |this, cx| this.dismiss_project_trust(window, cx));
        },
        |surface| {
            let mut choices = div().flex().flex_col().gap(THEME.space.xs);
            for (index, option) in project_trust::options(project)
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
                    .gap(THEME.space.md)
                    .p(THEME.space.md)
                    .child(
                        div()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.body_small)
                            .text_color(THEME.colors.accent)
                            .child(project.display().to_string()),
                    )
                    .child(
                        div()
                            .text_color(THEME.colors.muted)
                            .line_height(THEME.type_scale.line_body)
                            .child("Trusting allows Pi to load project settings and resources, install missing project packages, and execute project extensions."),
                    )
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(saved),
                    )
                    .when_some(app.project_trust_error.clone(), |content, error| {
                        content.child(
                            div()
                                .text_color(THEME.colors.error)
                                .child(format!("Trust decision was not saved: {error}")),
                        )
                    })
                    .child(choices)
                    .child(
                        div()
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.subtle)
                            .child(if app.pending_project_trust_command.is_some() {
                                "Choose a decision to continue opening this project."
                            } else {
                                "Restart Farcaster after changing this decision."
                            }),
                    ),
            )
        },
    )
    .into_any_element()
}
