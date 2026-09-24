use super::*;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let profiles = app.settings.harness_profiles.list().unwrap_or_default();
    let mut content = div()
        .flex()
        .flex_col()
        .gap(THEME.space.sm)
        .child(
            div()
                .text_size(THEME.type_scale.reading)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Harness profiles"),
        )
        .child(
            div()
                .text_size(THEME.type_scale.body_small)
                .text_color(THEME.colors.muted)
                .child("Run another command with a supported harness protocol."),
        );
    for profile in profiles {
        let remove = entity.clone();
        let profile_id = profile.id.clone();
        let detail = format!(
            "{} · {}{}",
            crate::agents::backend_display_name(profile.backend),
            profile.executable.display(),
            profile
                .data_directory
                .as_ref()
                .map_or(String::new(), |directory| format!(
                    " · {}",
                    directory.display()
                )),
        );
        content = content.child(
            div()
                .flex()
                .flex_col()
                .child(profile.name)
                .child(div().text_color(THEME.colors.muted).child(detail))
                .child(button(
                    format!("remove-profile-{profile_id}"),
                    "Remove",
                    ButtonTone::Quiet,
                    true,
                    move |_, cx| {
                        let _ = remove.update(cx, |this, cx| {
                            this.remove_harness_profile(profile_id.clone(), cx)
                        });
                    },
                )),
        );
    }
    let mut backends = div().flex().flex_wrap().gap(THEME.space.xs);
    for backend in crate::agents::Backend::ALL {
        let select = entity.clone();
        backends = backends.child(button(
            format!("profile-backend-{backend}"),
            crate::agents::backend_display_name(backend),
            if app.settings.harness_profile_backend == backend {
                ButtonTone::Neutral
            } else {
                ButtonTone::Quiet
            },
            true,
            move |_, cx| {
                let _ = select.update(cx, |this, cx| {
                    this.choose_harness_profile_backend(backend, cx)
                });
            },
        ));
    }
    let add = entity.clone();
    content
        .child(super::setting_label("Add profile", "Choose the protocol spoken by the command. Set its data directory if it stores sessions elsewhere."))
        .child(backends)
        .child(Input::new(&app.settings.harness_profile_name))
        .child(Input::new(&app.settings.harness_profile_executable))
        .when(crate::agents::profile_data_environment_key(app.settings.harness_profile_backend).is_some(), |content| {
            content.child(Input::new(&app.settings.harness_profile_data_directory))
        })
        .when_some(app.settings.harness_profile_error.clone(), |content, error| {
            content.child(feedback("harness-profile-error", error, FeedbackTone::Error))
        })
        .child(button(
            "add-harness-profile",
            "Add profile",
            ButtonTone::Neutral,
            true,
            move |window, cx| {
                let _ = add.update(cx, |this, cx| this.add_harness_profile(window, cx));
            },
        ))
        .into_any_element()
}
