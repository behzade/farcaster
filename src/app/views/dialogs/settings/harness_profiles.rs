use super::*;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let profiles = app.settings.harness_profiles.list().unwrap_or_default();
    let toggle = entity.clone();
    let adding = app.settings.adding_harness_profile;
    let toggle_label = if adding {
        "Close form"
    } else {
        "+ Add profile"
    };
    let mut content = div().flex().flex_col().gap(theme().space.sm).child(
        div()
            .flex()
            .items_center()
            .gap(theme().space.sm)
            .child(setting_label("Harness profiles", ""))
            .child(
                settings_control(
                    "toggle-harness-profile-form",
                    toggle_label,
                    &app.settings.harness_form_focus,
                )
                .debug_selector(|| "toggle-harness-profile-form".into())
                .aria_expanded(adding)
                .w_auto()
                .px(theme().space.sm)
                .child(if adding { "Close form" } else { "Add profile" })
                .child(app_icon(
                    if adding { AppIcon::X } else { AppIcon::Plus },
                    AppIconSize::Control,
                ))
                .on_click(move |_, window, cx| {
                    let _ = toggle.update(cx, |this, cx| {
                        this.settings.adding_harness_profile = !adding;
                        this.settings.harness_form_focus.focus(window, cx);
                        cx.notify();
                    });
                }),
            ),
    ).child(
        div().text_size(theme().type_scale.body_small).text_color(theme().colors.muted)
            .child("Separate commands and data directories for agent connections. Worker profiles choose the model and effort.")
    );
    let empty = profiles.is_empty();
    let mut list = div()
        .id("harness-profile-list")
        .max_h(theme().size(180.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(theme().space.sm);
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
                ))
        );
        list = list.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(profile.name)
                        .child(
                            div()
                                .truncate()
                                .text_color(theme().colors.muted)
                                .child(detail),
                        ),
                )
                .child(settings_action(
                    format!("remove-profile-{profile_id}"),
                    "Remove harness profile",
                    AppIcon::Trash,
                    true,
                    move |_, cx| {
                        let _ = remove.update(cx, |this, cx| {
                            this.remove_harness_profile(profile_id.clone(), cx)
                        });
                    },
                )),
        );
    }
    content = content
        .when(empty && !adding, |content| {
            content.child(
                div()
                    .text_color(theme().colors.muted)
                    .child("No connection profiles yet."),
            )
        })
        .when(!empty, |content| content.child(list));
    if adding {
        content = content.child(add_form(app, entity));
    }
    content
        .when_some(
            app.settings.harness_profile_error.clone(),
            |content, error| {
                content.child(feedback(
                    "harness-profile-error",
                    error,
                    FeedbackTone::Error,
                ))
            },
        )
        .into_any_element()
}

fn add_form(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let mut backends = div().flex().flex_wrap().gap(theme().space.xs);
    for backend in crate::agents::Backend::ALL {
        let select = entity.clone();
        let selected = app.settings.harness_profile_backend == backend;
        backends = backends.child(
            button(
                format!("profile-backend-{backend}"),
                crate::agents::backend_display_name(backend),
                ButtonTone::Quiet,
                true,
                move |window, cx| {
                    let _ = select.update(cx, |this, cx| {
                        this.choose_harness_profile_backend(backend, window, cx)
                    });
                },
            )
            .selected(selected)
            .toggled(selected)
            .selection_style(selected),
        );
    }
    div()
        .debug_selector(|| "harness-profile-form".into())
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .p(theme().space.md)
        .rounded(theme().size(6.0))
        .bg(theme().colors.surface)
        .child(setting_label("Add profile", ""))
        .child(backends)
        .child(
            div()
                .flex()
                .gap(theme().space.sm)
                .child(field("Name", &app.settings.harness_profile_name))
                .child(field(
                    "Command or absolute path",
                    &app.settings.harness_profile_executable,
                )),
        )
        .when(
            crate::agents::profile_data_environment_key(app.settings.harness_profile_backend)
                .is_some(),
            |form| {
                form.child(field(
                    "Data directory (optional)",
                    &app.settings.harness_profile_data_directory,
                ))
            },
        )
        .child(div().flex().justify_end().child(button(
            "add-harness-profile",
            "Add profile",
            ButtonTone::Accent,
            true,
            move |window, cx| {
                let _ = entity.update(cx, |this, cx| this.add_harness_profile(window, cx));
            },
        )))
        .into_any_element()
}

fn field(
    label: &'static str,
    input: &gpui::Entity<gpui_component::input::InputState>,
) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child(label),
        )
        .child(Input::new(input))
        .into_any_element()
}
