use super::*;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, app_icon},
    editors::{EditorIcon, text_editor_statuses},
};

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let selected = app.text_editor_name();
    let mut choices = div().flex().flex_col().gap(theme().space.xs).child(choice(
        "automatic",
        "Automatic",
        "Use the first editor found on PATH.",
        None,
        selected == "Editor",
        entity.clone(),
        None,
    ));
    for status in text_editor_statuses() {
        let detail = if status.available {
            status.program.clone()
        } else {
            format!("{} (not installed)", status.program)
        };
        choices = choices.child(choice(
            status.id,
            status.name,
            &detail,
            Some(status.icon),
            selected == status.name,
            entity.clone(),
            Some(status.program),
        ));
    }

    let clear = entity.clone();
    let use_command = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .pt(theme().space.md)
        .border_t_1()
        .border_color(theme().colors.surface)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme().space.xs)
                .child(
                    div()
                        .text_size(theme().type_scale.reading)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Editor"),
                )
                .child(
                    div()
                        .text_size(theme().type_scale.body_small)
                        .text_color(theme().colors.muted)
                        .child(
                            "The editor surface runs this command in the project. Opening files from the app needs Neovim.",
                        ),
                ),
        )
        .child(choices)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme().space.xs)
                .child(setting_label(
                    "Custom command",
                    "Any command with arguments, for example micro -p. Saves on Enter.",
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme().space.sm)
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&app.settings.text_editor_input)),
                        )
                        .child(button(
                            "save-text-editor",
                            "Use",
                            ButtonTone::Neutral,
                            true,
                            move |window, cx| {
                                let _ = use_command.update(cx, |this, cx| {
                                    this.save_settings_text_editor(window, cx)
                                });
                            },
                        ))
                        .child(button(
                            "clear-text-editor",
                            "Automatic",
                            ButtonTone::Quiet,
                            app.settings.text_editor.is_some(),
                            move |window, cx| {
                                let _ = clear.update(cx, |this, cx| {
                                    this.set_settings_text_editor(None, window, cx)
                                });
                            },
                        )),
                ),
        )
        .when_some(app.settings.text_editor_error.clone(), |section, error| {
            section.child(feedback(
                "settings-text-editor-error",
                error,
                FeedbackTone::Error,
            ))
        })
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn choice(
    id: &'static str,
    title: &'static str,
    detail: &str,
    icon: Option<EditorIcon>,
    selected: bool,
    entity: WeakEntity<FarcasterApp>,
    command: Option<String>,
) -> AnyElement {
    let mut row = div()
        .id((gpui::ElementId::from("settings-editor"), id))
        .role(gpui::Role::Button)
        .aria_label(format!("{title}: {detail}"))
        .tab_index(0)
        .flex()
        .items_center()
        .gap(theme().space.sm)
        .px(theme().space.sm)
        .py(theme().space.xs)
        .cursor_pointer()
        .when(selected, |row| row.bg(theme().colors.highlight))
        .hover(|row| row.bg(theme().colors.highlight))
        .on_click(move |_, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.set_settings_text_editor(command.clone(), window, cx);
            });
        })
        .child(
            div()
                .w(theme().size(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().colors.muted)
                .when_some(icon.map(AppIcon::for_editor), |slot, icon| {
                    slot.child(app_icon(icon, AppIconSize::Inline))
                }),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if selected {
                            theme().colors.text
                        } else {
                            theme().colors.muted
                        })
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child(detail.to_owned()),
                ),
        );
    if selected {
        row = row.child(
            app_icon(AppIcon::Check, AppIconSize::Inline).text_color(theme().colors.indicator),
        );
    }
    row.into_any_element()
}
