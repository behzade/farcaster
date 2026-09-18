use super::*;
use crate::app::{
    ui::theme::{Appearance, LengthKey, ThemeToken, length_label, token_label},
    workspace::theme_settings::ThemeSettings,
};
use gpui::SharedString;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let themes = &app.settings.themes;
    let selected = themes.library.selected_name().to_owned();
    let editable = themes.editable();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme().space.xs)
                .child(
                    div()
                        .text_size(theme().type_scale.reading)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Appearance"),
                )
                .child(
                    div()
                        .text_size(theme().type_scale.body_small)
                        .text_color(theme().colors.muted)
                        .child(
                            "Themes apply to the whole app. Duplicate one to change its colors.",
                        ),
                ),
        )
        .child(theme_list(themes, &selected, editable, entity.clone()))
        .child(theme_actions(themes, editable, entity.clone()))
        .when_some(theme_editor(themes, editable, entity), |section, editor| {
            section.child(editor)
        })
        .when_some(themes.error.clone(), |section, error| {
            section.child(feedback("settings-theme-error", error, FeedbackTone::Error))
        })
        .when_some(themes.status.clone(), |section, status| {
            section.child(feedback(
                "settings-theme-status",
                status,
                FeedbackTone::Info,
            ))
        })
        .into_any_element()
}

fn theme_list(
    themes: &ThemeSettings,
    selected: &str,
    editable: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let mut list = div()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .rounded(theme().radius)
        .border(theme().border)
        .border_color(theme().colors.surface)
        .p(theme().space.sm);
    for (index, definition) in themes.library.display_order().into_iter().enumerate() {
        let active = definition.name == selected;
        let custom = themes.library.is_user_theme(&definition.name);
        let select = entity.clone();
        let select_name = definition.name.clone();
        let duplicate = entity.clone();
        let duplicate_name = definition.name.clone();
        list = list.child(
            div()
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
                                .text_size(theme().type_scale.body)
                                .text_color(theme().colors.text)
                                .child(definition.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(theme().type_scale.caption)
                                .text_color(theme().colors.muted)
                                .child(if custom { "Your theme" } else { "Built in" }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(theme().space.xs)
                        .child(button(
                            ("theme-duplicate", index),
                            "Duplicate",
                            ButtonTone::Quiet,
                            editable,
                            move |window, cx| {
                                let _ = duplicate.update(cx, |this, cx| {
                                    this.create_theme_from(&duplicate_name, window, cx)
                                });
                            },
                        ))
                        .child(button(
                            ("theme-select", index),
                            if active { "Active" } else { "Use" },
                            if active {
                                ButtonTone::Accent
                            } else {
                                ButtonTone::Neutral
                            },
                            editable && !active,
                            move |window, cx| {
                                let _ = select.update(cx, |this, cx| {
                                    this.select_theme(&select_name, window, cx)
                                });
                            },
                        )),
                ),
        );
    }
    list.into_any_element()
}

fn theme_actions(
    themes: &ThemeSettings,
    editable: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let export = entity.clone();
    let import = entity.clone();
    let delete = entity;
    div()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .child(button(
            "theme-export",
            "Export…",
            ButtonTone::Neutral,
            editable,
            move |window, cx| {
                let _ = export.update(cx, |this, cx| this.export_theme(window, cx));
            },
        ))
        .child(button(
            "theme-import",
            "Import…",
            ButtonTone::Neutral,
            editable,
            move |window, cx| {
                let _ = import.update(cx, |this, cx| this.import_theme(window, cx));
            },
        ))
        .child(button(
            "theme-delete",
            "Delete",
            ButtonTone::Danger,
            editable && themes.draft.is_some(),
            move |window, cx| {
                let _ = delete.update(cx, |this, cx| this.delete_theme(window, cx));
            },
        ))
        .child(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child("Export writes the active theme to a CSS file."),
        )
        .into_any_element()
}

fn theme_editor(
    themes: &ThemeSettings,
    editable: bool,
    entity: WeakEntity<FarcasterApp>,
) -> Option<AnyElement> {
    let draft = themes.draft.as_ref()?;
    let mut editor = div()
        .flex()
        .flex_col()
        .gap(theme().space.sm)
        .border_t_1()
        .border_color(theme().colors.surface)
        .pt(theme().space.md)
        .child(
            div()
                .text_size(theme().type_scale.body)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme().colors.text)
                .child(format!("Editing {}", draft.name)),
        );
    if let Some(input) = themes.name.clone() {
        editor = editor.child(editor_row(
            "Name",
            None,
            false,
            div().flex_1().child(Input::new(&input)),
        ));
    }
    editor = editor.child(editor_row(
        "Appearance",
        None,
        false,
        div().flex().items_center().gap(theme().space.xs).children(
            [Appearance::Dark, Appearance::Light]
                .into_iter()
                .enumerate()
                .map(|(index, appearance)| {
                    let select = entity.clone();
                    let label = appearance_label(appearance);
                    let selected = draft.appearance == appearance;
                    button(
                        ("theme-appearance", index),
                        label,
                        ButtonTone::Quiet,
                        editable,
                        move |_, cx| {
                            let _ = select
                                .update(cx, |this, cx| this.set_theme_appearance(appearance, cx));
                        },
                    )
                    .toggled(selected)
                }),
        ),
    ));
    let mut group = None;
    for (token, input) in &themes.tokens {
        let next = token_group(*token);
        if group != Some(next) {
            editor = editor.child(group_header(next));
            group = Some(next);
        }
        editor = editor.child(editor_row(
            token_label(*token),
            Some(draft.color(*token)),
            draft.is_custom(*token),
            div()
                .w(theme().size(132.0))
                .flex_none()
                .child(Input::new(input)),
        ));
    }
    let mut group = None;
    for (key, input) in &themes.lengths {
        let next = length_group(*key);
        if group != Some(next) {
            editor = editor.child(group_header(next));
            group = Some(next);
        }
        editor = editor.child(editor_row(
            length_label(*key),
            None,
            draft.is_custom_length(*key),
            div()
                .w(theme().size(132.0))
                .flex_none()
                .child(Input::new(input)),
        ));
    }
    Some(editor.into_any_element())
}

fn length_group(key: LengthKey) -> &'static str {
    match key {
        LengthKey::Metric(_) => "Metrics",
        LengthKey::Size(_) => "Sizes",
    }
}

fn token_group(token: ThemeToken) -> &'static str {
    match token {
        ThemeToken::Palette(_) => "Palette",
        ThemeToken::Icon(_) => "File icons",
        ThemeToken::Syntax(_) => "Code highlighting",
    }
}

fn group_header(label: &'static str) -> AnyElement {
    div()
        .pt(theme().space.sm)
        .text_size(theme().type_scale.body_small)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(theme().colors.muted)
        .child(label)
        .into_any_element()
}

fn editor_row(
    label: impl Into<SharedString>,
    swatch: Option<gpui::Rgba>,
    custom: bool,
    control: gpui::Div,
) -> AnyElement {
    let mut row = div().flex().items_center().gap(theme().space.sm).child(
        div()
            .w(theme().size(116.0))
            .flex_none()
            .text_size(theme().type_scale.body_small)
            .text_color(theme().colors.muted)
            .child(label.into()),
    );
    if let Some(color) = swatch {
        row = row.child(
            div()
                .w(theme().size(18.0))
                .h(theme().size(18.0))
                .flex_none()
                .rounded(theme().radius)
                .border(theme().border)
                .border_color(theme().colors.border)
                .bg(color),
        );
    }
    row.child(control)
        .when(custom, |row| {
            row.child(
                div()
                    .flex_none()
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .child("custom"),
            )
        })
        .into_any_element()
}

fn appearance_label(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Dark => "Dark",
        Appearance::Light => "Light",
    }
}
