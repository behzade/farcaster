use super::*;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let mut rows = div()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .child(setting_label(
            "Harness sign-in",
            "Sign in with your account to use a harness.",
        ));
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
    for (backend, profile_id, name) in targets {
        let sign_in = entity.clone();
        rows = rows.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(name)
                .child(button(
                    format!(
                        "harness-sign-in-{backend}-{}",
                        profile_id.as_deref().unwrap_or("default")
                    ),
                    "Sign in",
                    ButtonTone::Quiet,
                    app.settings.sign_in.active.is_none(),
                    move |_, cx| {
                        let _ = sign_in.update(cx, |this, cx| {
                            this.sign_in_harness(backend, profile_id.clone(), cx)
                        });
                    },
                )),
        );
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
    if let Some(url) = app.settings.sign_in.url.clone() {
        rows = rows.child(button(
            "harness-sign-in-browser",
            "Open sign-in page",
            ButtonTone::Quiet,
            true,
            move |_, cx| cx.open_url(&url),
        ));
    }
    if app.settings.sign_in.active.is_some() {
        rows = rows.child(button(
            "harness-sign-in-cancel",
            "Cancel",
            ButtonTone::Quiet,
            true,
            move |_, cx| {
                let _ = entity.update(cx, FarcasterApp::cancel_harness_sign_in);
            },
        ));
    }
    rows.into_any_element()
}
