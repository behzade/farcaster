use super::*;

pub(in crate::app::views) fn render_terminal(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let Some(terminal) = &app.settings.sign_in.terminal else {
        return div().into_any_element();
    };
    div()
        .debug_selector(|| "settings-sign-in-terminal".into())
        .w_full()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(
            div()
                .flex_none()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(theme().space.xs)
                        .child(
                            div()
                                .text_size(theme().type_scale.reading)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Terminal login"),
                        )
                        .child(
                            div()
                                .text_size(theme().type_scale.body_small)
                                .text_color(theme().colors.muted)
                                .child("Complete sign-in in the terminal below. This closes automatically when login finishes."),
                        ),
                )
                .child(
                    button(
                        "harness-sign-in-cancel",
                        "Cancel",
                        ButtonTone::Neutral,
                        true,
                        move |_, cx| {
                            let _ = entity.update(cx, FarcasterApp::cancel_harness_sign_in);
                        },
                    )
                    .flex_none()
                    .debug_selector(|| "harness-sign-in-cancel".into()),
                ),
        )
        .child(
            div()
                .debug_selector(|| "harness-sign-in-terminal-frame".into())
                .w_full()
                .flex_1()
                .min_h_0()
                .rounded(theme().size(6.0))
                .overflow_hidden()
                .border_1()
                .border_color(theme().colors.border)
                .child(terminal.clone()),
        )
        .into_any_element()
}

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let sign_in_enabled = app.settings.sign_in.active.is_none()
        && app.settings.sign_in.terminal.is_none()
        && !app.settings.sign_in.signing_out;
    let mut rows = div()
        .debug_selector(|| "settings-sign-in".into())
        .w_full()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme().space.xs)
                .child(
                    div()
                        .text_size(theme().type_scale.reading)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Sign in"),
                )
                .child(
                    div()
                        .text_size(theme().type_scale.body_small)
                        .text_color(theme().colors.muted)
                        .child("Sign in to a local agent CLI to connect its account."),
                ),
        );
    let mut targets = crate::agents::Backend::ALL
        .into_iter()
        .filter(|backend| crate::agents::supports_sign_in(*backend))
        .map(|backend| (backend, None, crate::agents::backend_display_name(backend)))
        .collect::<Vec<_>>();
    targets.extend(
        app.settings
            .harness_profiles
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|profile| crate::agents::supports_sign_in(profile.backend))
            .map(|profile| (profile.backend, Some(profile.id), profile.name)),
    );
    for (index, (backend, profile_id, name)) in targets.into_iter().enumerate() {
        let methods = crate::agents::sign_in_methods(backend);
        let title = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(theme().space.sm)
            .child(crate::app::ui::grid::icon_slot().child(app_icon(
                AppIcon::for_harness(backend),
                AppIconSize::Control,
            )))
            .child(
                div()
                    .text_size(theme().type_scale.body)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme().colors.text)
                    .child(name),
            )
            .when(profile_id.is_some(), |header| {
                header.child(
                    div()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.muted)
                        .child(crate::agents::backend_display_name(backend)),
                )
            });
        let make_button = |(method, label): (&'static str, &'static str)| {
            let sign_in = entity.clone();
            let profile_id = profile_id.clone();
            button(
                format!(
                    "harness-sign-in-{backend}-{}-{method}",
                    profile_id.as_deref().unwrap_or("default")
                ),
                label,
                ButtonTone::Neutral,
                sign_in_enabled,
                move |window, cx| {
                    let _ = sign_in.update(cx, |this, cx| {
                        this.sign_in_harness(backend, profile_id.clone(), method, window, cx)
                    });
                },
            )
            .shadow(crate::app::ui::primitives::control_shadow())
        };
        let card = div()
            .debug_selector(move || format!("sign-in-card-{index}"))
            .w_full()
            .p(theme().size(12.0))
            .rounded(theme().size(6.0))
            .bg(theme().colors.surface);
        let card = if let [only] = methods {
            card.flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(title)
                .child(make_button(*only).flex_none())
        } else {
            card.flex()
                .flex_col()
                .gap(theme().space.sm)
                .child(title)
                .child(
                    div()
                        .w_full()
                        .pl(theme().icons.control + theme().space.sm)
                        .flex()
                        .flex_wrap()
                        .gap(theme().space.sm)
                        .children(methods.iter().copied().map(make_button)),
                )
        };
        rows = rows.child(card);
    }
    if let Some(message) = &app.settings.sign_in.message {
        rows = rows.child(feedback(
            "harness-sign-in-status",
            message.clone(),
            if app.settings.sign_in.failed {
                FeedbackTone::Error
            } else {
                FeedbackTone::Info
            },
        ));
    }
    let mut actions: Vec<AnyElement> = Vec::new();
    if let Some(url) = app.settings.sign_in.url.clone() {
        actions.push(
            button(
                "harness-sign-in-browser",
                "Open sign-in page",
                ButtonTone::Neutral,
                true,
                move |_, cx| cx.open_url(&url),
            )
            .icon(AppIcon::ArrowSquareOut)
            .into_any_element(),
        );
    }
    if app.settings.sign_in.active.is_some() {
        actions.push(
            button(
                "harness-sign-in-cancel",
                "Cancel",
                ButtonTone::Neutral,
                true,
                move |_, cx| {
                    let _ = entity.update(cx, FarcasterApp::cancel_harness_sign_in);
                },
            )
            .into_any_element(),
        );
    }
    if !actions.is_empty() {
        rows = rows.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(theme().space.sm)
                .children(actions),
        );
    }
    rows.into_any_element()
}
