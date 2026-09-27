mod appearance;
mod harness_profiles;
mod worker_tasks;
use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    Disableable as _, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    input::Input,
};

use super::super::FarcasterApp;
use crate::{
    app::OVERLAY_KEY_CONTEXT,
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, FeedbackTone, app_icon, button, feedback, modal,
    },
    app::ui::theme::theme,
    app::workspace::{
        SettingsTab,
        editor::{editor_available, effective_editor_choice},
    },
    storage::EditorChoice,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    cx: &gpui::App,
) -> AnyElement {
    let dismiss = entity.clone();
    let tab = app.settings.tab;
    let content = match tab {
        SettingsTab::General => general(app, entity.clone(), cx),
        SettingsTab::Workers => worker_tasks::render(app, entity.clone(), cx),
        SettingsTab::Appearance => appearance::render(app, entity.clone()),
        SettingsTab::Connections => connections(app, entity.clone()),
    };
    modal(
        "settings",
        "Settings",
        &app.overlays.sheet_focus,
        OVERLAY_KEY_CONTEXT,
        move |window, cx| {
            let _ = dismiss.update(cx, |this, cx| this.close_sheet(window, cx));
        },
        |surface| {
            let close = entity.clone();
            surface
                .w(theme().size(860.0))
                .max_w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .flex_wrap()
                        .gap(theme().space.sm)
                        .px(theme().size(24.0))
                        .py(theme().space.md)
                        .border_b_1()
                        .border_color(theme().colors.surface)
                        .child(
                            div()
                                .text_size(theme().type_scale.display)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Settings"),
                        )
                        .child(div().flex().gap(theme().space.xs).children(
                            SettingsTab::ALL.into_iter().map(|page| {
                                let select = entity.clone();
                                let selected = page == tab;
                                settings_control(
                                    format!("settings-tab-{}", page.label()),
                                    page.label(),
                                    &app.settings.tab_focus[page as usize],
                                )
                                .debug_selector(move || format!("settings-tab-{}", page.label()))
                                .w_auto()
                                .px(theme().space.sm)
                                .aria_selected(selected)
                                .text_color(if selected {
                                    theme().colors.indicator
                                } else {
                                    theme().colors.muted
                                })
                                .when(selected, |tab| tab.bg(theme().colors.highlight))
                                .child(page.label())
                                .on_click(move |_, window, cx| {
                                    let _ = select.update(cx, |this, cx| {
                                        this.settings.tab = page;
                                        this.settings.tab_focus[page as usize].focus(window, cx);
                                        cx.notify();
                                    });
                                })
                            }),
                        )),
                )
                .child(
                    div()
                        .id(format!(
                            "settings-scroll-{}-{}",
                            tab.label(),
                            app.settings.themes.editing
                        ))
                        .debug_selector(|| "settings-content".into())
                        .min_h_0()
                        .h(theme().size(520.0))
                        .max_h(theme().size(520.0))
                        .overflow_y_scroll()
                        .p(theme().size(24.0))
                        .child(content),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(theme().space.sm)
                        .px(theme().size(24.0))
                        .py(theme().space.md)
                        .border_t_1()
                        .border_color(theme().colors.surface)
                        .child(
                            div()
                                .text_size(theme().type_scale.caption)
                                .text_color(theme().colors.muted)
                                .child("Valid changes save automatically."),
                        )
                        .child(
                            settings_action(
                                "close-settings",
                                "Close settings",
                                AppIcon::X,
                                true,
                                move |window, cx| {
                                    let _ =
                                        close.update(cx, |this, cx| this.close_sheet(window, cx));
                                },
                            )
                            .debug_selector(|| "close-settings".into()),
                        ),
                )
        },
    )
    .into_any_element()
}

fn settings_control(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    focus: &gpui::FocusHandle,
) -> gpui::Stateful<gpui::Div> {
    crate::app::ui::primitives::icon_control(id, label)
        .track_focus(focus)
        .hover(|control| control.bg(theme().colors.highlight))
}

fn settings_action(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    icon: AppIcon,
    enabled: bool,
    on_press: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> Button {
    let label = label.into();
    Button::new(id)
        .accessibility_label(label.clone())
        .tooltip(label)
        .ghost()
        .with_size(Size::Small)
        .size(theme().controls.icon_button)
        .disabled(!enabled)
        .child(app_icon(icon, AppIconSize::Control))
        .on_click(move |_, window, cx| on_press(window, cx))
}

fn general(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>, cx: &gpui::App) -> AnyElement {
    div()
        .debug_selector(|| "settings-general".into())
        .flex()
        .flex_col()
        .gap(theme().size(24.0))
        .child(editor_setting(app, entity.clone()))
        .when_some(app.settings.editor_error.clone(), |content, error| {
            content.child(feedback(
                "settings-editor-error",
                error,
                FeedbackTone::Error,
            ))
        })
        .child(toggle_setting(
            "session-project-groups-toggle",
            "Group sessions by project",
            "Group all chats by project. Turn off to use custom folders.",
            app.settings.group_sessions_by_project,
            entity.clone(),
            FarcasterApp::toggle_settings_project_groups,
        ))
        .when_some(
            app.settings.session_grouping_error.clone(),
            |content, error| {
                content.child(feedback(
                    "settings-session-grouping-error",
                    error,
                    FeedbackTone::Error,
                ))
            },
        )
        .child(transcript_font_size(
            app.views.transcript.read(cx).font_size,
            entity.clone(),
        ))
        .child(toggle_setting(
            "transcript-folders-toggle",
            "Expand changed folders in transcript",
            "Start change folders expanded. Your manual folder choices stay as you left them.",
            app.settings.expand_transcript_folders,
            entity.clone(),
            FarcasterApp::toggle_settings_transcript_folders,
        ))
        .when_some(app.settings.transcript_error.clone(), |content, error| {
            content.child(feedback(
                "settings-transcript-error",
                error,
                FeedbackTone::Error,
            ))
        })
        .into_any_element()
}

fn connections(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let clear = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(theme().size(24.0))
        .child(harness_profiles::render(app, entity.clone()))
        .child(toggle_setting(
            "builtin-mcp-toggle",
            "Built-in MCP",
            "Add local tools to new sessions. Turning this off disconnects existing MCP clients.",
            crate::builtin_mcp::enabled(),
            entity.clone(),
            FarcasterApp::toggle_settings_builtin_mcp,
        ))
        .when_some(app.settings.mcp_error.clone(), |content, error| {
            content.child(feedback("settings-mcp-error", error, FeedbackTone::Error))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme().space.sm)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(theme().space.md)
                        .child(setting_label(
                            "Network proxy",
                            "Used when the project environment has no HTTP or HTTPS proxy.",
                        ))
                        .child(
                            div()
                                .w(theme().size(300.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(theme().space.sm)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&app.settings.network_proxy_input)),
                                )
                                .child(settings_action(
                                    "clear-network-proxy",
                                    "Clear network proxy",
                                    AppIcon::Eraser,
                                    true,
                                    move |window, cx| {
                                        let _ = clear.update(cx, |this, cx| {
                                            this.clear_network_proxy(window, cx)
                                        });
                                    },
                                )),
                        ),
                )
                .when_some(
                    app.settings.network_proxy_error.clone(),
                    |content, error| {
                        content.child(feedback("settings-proxy-error", error, FeedbackTone::Error))
                    },
                ),
        )
        .into_any_element()
}

fn editor_setting(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let choice = app.settings.editor_choice;
    let project = app.workspace_project();
    let selected = effective_editor_choice(choice, &project);
    let available: Vec<_> = EditorChoice::ALL
        .into_iter()
        .filter(|choice| *choice == EditorChoice::Custom || editor_available(*choice, &project))
        .collect();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(setting_label(
            "Editor",
            "Open files, projects, and review locations in this editor. Chat code capture uses embedded Neovim.",
        ))
        .child(
            div()
                .flex()
                .gap(theme().space.xs)
                .flex_wrap()
                .children(available.into_iter().map(|option| {
                    editor_option(
                        format!("editor-{}", option.as_str()),
                        option.label(),
                        AppIcon::for_editor(option),
                        option,
                        selected,
                        entity.clone(),
                    )
                })),
        )
        .when(choice == EditorChoice::Custom, |section| {
            section.child(setting_label("Terminal command", "Command and arguments, for example micro -softwrap true. Quote paths containing spaces. Applies to new editor sessions."))
                .child(Input::new(&app.settings.editor_command_input))
        })
        .into_any_element()
}

fn editor_option(
    id: String,
    label: &'static str,
    icon: AppIcon,
    option: EditorChoice,
    selected: EditorChoice,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    Button::new(id)
        .icon(icon)
        .label(label)
        .with_size(Size::Small)
        .toggled(option == selected)
        .when(option == selected, |button| button.primary())
        .when(option != selected, |button| button.secondary())
        .on_click(move |_, _, cx| {
            let _ = entity.update(cx, |this, cx| this.select_editor(option, cx));
        })
        .into_any_element()
}

fn toggle_setting(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    enabled: bool,
    entity: WeakEntity<FarcasterApp>,
    toggle: fn(&mut FarcasterApp, &mut gpui::Context<FarcasterApp>),
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(theme().space.md)
        .child(setting_label(title, description))
        .child(
            Button::new(id)
                .label(if enabled { "On" } else { "Off" })
                .with_size(Size::Small)
                .toggled(enabled)
                .when(enabled, |button| button.primary())
                .when(!enabled, |button| button.secondary())
                .on_click(move |_, _, cx| {
                    let _ = entity.update(cx, toggle);
                }),
        )
        .into_any_element()
}

fn setting_label(title: &'static str, description: &'static str) -> AnyElement {
    div()
        .min_w_0()
        .max_w(theme().size(480.0))
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .text_size(theme().type_scale.body)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme().colors.text)
                .child(title),
        )
        .child(
            div()
                .text_size(theme().type_scale.body_small)
                .text_color(theme().colors.muted)
                .child(description),
        )
        .into_any_element()
}

fn transcript_font_size(size: gpui::Pixels, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    use crate::app::ui::theme::TRANSCRIPT_FONT_SIZE_RANGE;

    let size = f32::from(size);
    div()
        .debug_selector(|| "settings-font-size".into())
        .flex()
        .items_center()
        .justify_between()
        .gap(theme().space.md)
        .child(setting_label(
            "Transcript font size",
            if cfg!(target_os = "macos") {
                "Applies to all sessions. Cmd+- / Cmd+= also adjust the size."
            } else {
                "Applies to all sessions. Ctrl+- / Ctrl+= also adjust the size."
            },
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .child(format!("{size} px"))
                .children(
                    [
                        (
                            "transcript-font-smaller",
                            "Decrease transcript font size",
                            AppIcon::Minus,
                            size - 1.0,
                            size > *TRANSCRIPT_FONT_SIZE_RANGE.start(),
                        ),
                        (
                            "transcript-font-larger",
                            "Increase transcript font size",
                            AppIcon::Plus,
                            size + 1.0,
                            size < *TRANSCRIPT_FONT_SIZE_RANGE.end(),
                        ),
                        (
                            "transcript-font-reset",
                            "Reset transcript font size",
                            AppIcon::ArrowCounterClockwise,
                            f32::from(theme().type_scale.reading),
                            size != f32::from(theme().type_scale.reading),
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, icon, next, enabled)| {
                        let entity = entity.clone();
                        settings_action(id, label, icon, enabled, move |_, cx| {
                            let _ = entity
                                .update(cx, |this, cx| this.set_transcript_font_size(next, cx));
                        })
                    }),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
