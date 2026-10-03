use super::*;
use crate::app::{
    ui::theme::{Appearance, Colors, LengthKey, ThemeToken, length_label, token_label},
    workspace::theme_settings::ThemeSettings,
};
use gpui::SharedString;
use gpui_component::Selectable as _;

pub(super) fn render(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let themes = &app.settings.themes;
    let selected = themes.library.selected_name();
    let editable = themes.editable();
    let editing = themes.editing && themes.draft.is_some();
    let toggle = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(theme().space.sm)
                .child(
                    div()
                        .text_size(theme().type_scale.reading)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(if editing {
                            format!("Editing {selected}")
                        } else {
                            "Themes".into()
                        }),
                )
                .when(themes.draft.is_some() && editable, |header| {
                    header.child(
                        settings_control(
                            "theme-editor-toggle",
                            if editing {
                                "Back to themes"
                            } else {
                                "Edit theme"
                            },
                            &app.settings.theme_editor_focus,
                        )
                        .debug_selector(|| "theme-editor-toggle".into())
                        .child(app_icon(
                            if editing {
                                AppIcon::ArrowLeft
                            } else {
                                AppIcon::PencilSimple
                            },
                            AppIconSize::Control,
                        ))
                        .on_click(move |_, window, cx| {
                            let _ = toggle.update(cx, |this, cx| {
                                this.settings.themes.editing = !editing;
                                this.settings.theme_editor_focus.focus(window, cx);
                                cx.notify();
                            });
                        }),
                    )
                }),
        )
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
        .map(|section| {
            if editing {
                section.children(theme_editor(themes, editable, entity))
            } else {
                section
                    .child(theme_list(themes, entity.clone()))
                    .child(theme_actions(themes, editable, entity))
            }
        })
        .into_any_element()
}

fn theme_list(themes: &ThemeSettings, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let selected = themes.library.selected_name();
    let editable = themes.editable();
    let mut list = div()
        .debug_selector(|| "settings-theme-list".into())
        .flex()
        .flex_col()
        .gap(theme().space.xs);
    for (index, definition) in themes.library.display_order().into_iter().enumerate() {
        let active = definition.name == selected;
        let custom = themes.library.is_user_theme(&definition.name);
        let select = entity.clone();
        let select_name = definition.name.clone();
        list = list.child(
            div()
                .id(("theme-select", index))
                .debug_selector(move || format!("theme-select-{index}"))
                .px(theme().space.sm)
                .py(theme().space.sm)
                .border_1()
                .border_color(gpui::rgba(0x00000000))
                .when(active, |row| row.bg(theme().colors.highlight))
                .when(editable, |row| {
                    row.role(gpui::Role::Button)
                        .aria_label(definition.name.clone())
                        .aria_toggled(if active {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .tab_index(0)
                        .cursor_pointer()
                        .when(!active, |row| {
                            row.hover(|row| row.bg(theme().colors.surface))
                        })
                        .focus_visible(|row| row.border_color(theme().colors.accent))
                        .on_click(move |_, window, cx| {
                            if !active {
                                let _ = select.update(cx, |this, cx| {
                                    this.select_theme(&select_name, window, cx)
                                });
                            }
                        })
                })
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
                                .flex()
                                .items_center()
                                .gap(theme().space.sm)
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(theme().type_scale.body)
                                        .text_color(theme().colors.text)
                                        .child(definition.name.clone()),
                                )
                                .child(theme_palette(index, definition.colors)),
                        )
                        .child(
                            div()
                                .text_size(theme().type_scale.caption)
                                .text_color(theme().colors.muted)
                                .child(if custom { "Your theme" } else { "Built in" }),
                        ),
                )
                .when(active, |row| {
                    row.child(
                        div()
                            .flex_none()
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.indicator)
                            .child("Active"),
                    )
                }),
        );
    }
    list.into_any_element()
}

fn theme_palette(index: usize, colors: Colors) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("theme-palette", index))
        .flex()
        .flex_none()
        .gap(theme().size(2.0))
        .app_tooltip("Background · Text · Accent · Links · Success · Warning · Error")
        .children(
            [
                colors.canvas,
                colors.text,
                colors.accent,
                colors.link,
                colors.success,
                colors.warning,
                colors.error,
            ]
            .into_iter()
            .map(|color| {
                div()
                    .size(theme().size(12.0))
                    .flex_none()
                    .border_1()
                    .border_color(theme().colors.border)
                    .bg(color)
            }),
        )
}

fn theme_actions(
    themes: &ThemeSettings,
    editable: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let export = entity.clone();
    let import = entity.clone();
    let duplicate = entity.clone();
    let duplicate_name = themes.library.selected_name().to_owned();
    let delete = entity;
    div()
        .flex()
        .items_center()
        .gap(theme().space.xs)
        .flex_wrap()
        .child(settings_action(
            "theme-export",
            "Export active theme as CSS",
            AppIcon::DownloadSimple,
            editable,
            move |window, cx| {
                let _ = export.update(cx, |this, cx| this.export_theme(window, cx));
            },
        ))
        .child(settings_action(
            "theme-import",
            "Import theme from CSS",
            AppIcon::UploadSimple,
            editable,
            move |window, cx| {
                let _ = import.update(cx, |this, cx| this.import_theme(window, cx));
            },
        ))
        .child(settings_action(
            "theme-duplicate",
            "Duplicate to edit",
            AppIcon::Copy,
            editable,
            move |window, cx| {
                let _ = duplicate.update(cx, |this, cx| {
                    this.create_theme_from(&duplicate_name, window, cx)
                });
            },
        ))
        .when(themes.draft.is_some(), |actions| {
            actions.child(
                settings_action(
                    "theme-delete",
                    "Delete theme",
                    AppIcon::Trash,
                    editable,
                    move |window, cx| {
                        let _ = delete.update(cx, |this, cx| this.delete_theme(window, cx));
                    },
                )
                .text_color(theme().colors.danger),
            )
        })
        .into_any_element()
}

fn theme_editor(
    themes: &ThemeSettings,
    editable: bool,
    entity: WeakEntity<FarcasterApp>,
) -> Option<AnyElement> {
    let draft = themes.draft.as_ref()?;
    let mut editor = div()
        .debug_selector(|| "settings-theme-editor".into())
        .flex()
        .flex_col()
        .gap(theme().space.sm);
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
                    .selected(selected)
                    .when(selected, |button| {
                        button.text_color(theme().colors.indicator)
                    })
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
