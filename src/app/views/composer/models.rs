use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, ElementId, Focusable as _, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, deferred, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{input::Input, tooltip::Tooltip};

use super::footer::separator;
use crate::app::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, activates_button, app_icon, dropdown_content_button, icon_button,
    },
    app::ui::theme::{MONO_FONT_FAMILY, THEME},
    runtime::{ConfigurationStatus, HarnessAccessMode},
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let identity = app.snapshot.session_identity();
    let selected_model = identity.model;
    let selected_provider = identity.provider.map(str::to_owned).or_else(|| {
        app.snapshot
            .models
            .first()
            .map(|model| model.provider.clone())
    });
    let catalog_loading = selected_provider.is_none()
        && !app.snapshot.connected
        && app.snapshot.configuration_status == ConfigurationStatus::Loading;
    let provider_label = selected_provider.unwrap_or_else(|| "Provider".into());
    let model_label = selected_model.map_or_else(
        || {
            if catalog_loading {
                "Loading models…".into()
            } else {
                "Model".into()
            }
        },
        |model| model.name.clone(),
    );
    let effort = identity.effort.unwrap_or("off");
    let shows_effort = !app.snapshot.available_thinking_levels().is_empty() || effort != "off";
    let runtime_content = div()
        .flex()
        .items_center()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .child(div().text_color(THEME.colors.muted).child(provider_label))
        .child(runtime_slash())
        .child(div().text_color(THEME.colors.text).child(model_label))
        .when(shows_effort, |content| {
            content.child(runtime_slash()).child(
                div()
                    .text_color(effort_color(effort))
                    .child(effort_label(effort)),
            )
        });
    let runtime_entity = entity.clone();
    let runtime = dropdown_content_button(
        "select-runtime",
        "Runtime",
        runtime_content,
        ButtonTone::Neutral,
        true,
    )
    .flex_none()
    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .on_click(move |_, window, cx| {
        cx.stop_propagation();
        let _ = runtime_entity.update(cx, |this, cx| {
            this.runtime_picker_open = !this.runtime_picker_open;
            if this.runtime_picker_open {
                this.runtime_model_search
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            this.notify_composer(cx);
        });
    });

    div()
        .flex_none()
        .flex()
        .items_center()
        .child(runtime)
        .child(separator())
        .child(access_selector(
            app.snapshot.access_mode,
            &app.snapshot.harness,
            entity,
        ))
        .into_any_element()
}

pub(in crate::app::views) fn render_runtime_picker(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
    cx: &App,
) -> Option<AnyElement> {
    if !app.runtime_picker_open {
        return None;
    }
    let identity = app.snapshot.session_identity();
    let selected_provider = selected_provider(
        identity.provider,
        app.snapshot
            .models
            .first()
            .map(|model| model.provider.as_str()),
    );
    let selected_model = identity.model.map(|model| model.id.as_str());
    let selected_effort = identity.effort.unwrap_or("off");
    let efforts = app.snapshot.available_thinking_levels();
    let query = app
        .runtime_model_search
        .read(cx)
        .value()
        .trim()
        .to_ascii_lowercase();
    let mut providers = app
        .snapshot
        .models
        .iter()
        .map(|model| model.provider.clone())
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    let models = app
        .snapshot
        .models
        .iter()
        .filter(|model| model.provider == selected_provider)
        .filter(|model| model_matches_query(&model.name, &model.id, &query))
        .cloned()
        .collect::<Vec<_>>();
    let model_feedback =
        app.snapshot
            .models
            .is_empty()
            .then(|| match &app.snapshot.configuration_status {
                ConfigurationStatus::Loading if !app.snapshot.connected => {
                    "Refreshing models in the background…".to_owned()
                }
                ConfigurationStatus::Failed(error) => format!("Models unavailable: {error}"),
                ConfigurationStatus::Loading | ConfigurationStatus::Loaded => {
                    "No models were advertised by this harness.".to_owned()
                }
            });
    let close = entity.clone();

    Some(
        deferred(
            div()
                .id("runtime-picker")
                .absolute()
                .left(px(8.0))
                .bottom(px(43.0))
                .w(px(520.0))
                .max_w(gpui::relative(0.95))
                .p(px(10.0))
                .rounded(px(5.0))
                .border(THEME.border)
                .border_color(THEME.colors.border)
                .bg(THEME.colors.surface)
                .occlude()
                .shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(THEME.type_scale.caption)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(THEME.colors.text)
                                .child("Runtime"),
                        )
                        .child(icon_button(
                            "close-runtime-picker",
                            AppIcon::X,
                            "Close runtime picker",
                            ButtonTone::Quiet,
                            move |_, cx| {
                                let _ = close.update(cx, |this, cx| {
                                    this.runtime_picker_open = false;
                                    this.notify_composer(cx);
                                });
                            },
                        )),
                )
                .child(
                    div()
                        .h(px(300.0))
                        .flex()
                        .border(THEME.border)
                        .border_color(THEME.colors.hover)
                        .bg(THEME.colors.panel)
                        .child(
                            div()
                                .w(px(138.0))
                                .min_h_0()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .p(px(6.0))
                                .border_r(THEME.border)
                                .border_color(THEME.colors.hover)
                                .child(picker_label("Provider"))
                                .child(
                                    div()
                                        .id("runtime-provider-list")
                                        .min_h_0()
                                        .flex_1()
                                        .overflow_y_scroll()
                                        .children(providers.into_iter().enumerate().map(
                                            |(index, provider)| {
                                                let selected = provider == selected_provider;
                                                let target = provider.clone();
                                                let entity = entity.clone();
                                                runtime_option(
                                                    ("runtime-provider", index),
                                                    provider,
                                                    selected,
                                                    move |cx| {
                                                        let _ = entity.update(cx, |this, cx| {
                                                            this.select_provider(&target, cx);
                                                        });
                                                    },
                                                )
                                            },
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .min_h_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .p(px(6.0))
                                .child(picker_label("Model"))
                                .child(
                                    div()
                                        .h(px(28.0))
                                        .flex_none()
                                        .mb(px(5.0))
                                        .px(px(8.0))
                                        .rounded(px(3.0))
                                        .border(THEME.border)
                                        .border_color(THEME.colors.hover)
                                        .child(
                                            Input::new(&app.runtime_model_search)
                                                .w_full()
                                                .appearance(false),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("runtime-model-list")
                                        .min_h_0()
                                        .flex_1()
                                        .overflow_y_scroll()
                                        .when_some(model_feedback, |list, message| {
                                            list.child(picker_feedback(message))
                                        })
                                        .children(models.into_iter().enumerate().map(
                                            |(index, model)| {
                                                let selected =
                                                    selected_model == Some(model.id.as_str());
                                                let label = model.name.clone();
                                                let entity = entity.clone();
                                                runtime_option(
                                                    ("runtime-model", index),
                                                    label,
                                                    selected,
                                                    move |cx| {
                                                        let _ = entity.update(cx, |this, cx| {
                                                            this.select_model(&model, cx);
                                                        });
                                                    },
                                                )
                                            },
                                        )),
                                ),
                        ),
                )
                .when(!efforts.is_empty(), |picker| {
                    picker.child(
                        div()
                            .pt(px(9.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(picker_label("Effort"))
                            .children(efforts.iter().enumerate().map(|(index, effort)| {
                                let target = effort.clone();
                                let selected = effort == selected_effort;
                                let entity = entity.clone();
                                runtime_option(
                                    ("runtime-effort", index),
                                    effort_label(effort),
                                    selected,
                                    move |cx| {
                                        let _ = entity.update(cx, |this, cx| {
                                            this.set_thinking_level(target.clone(), cx);
                                        });
                                    },
                                )
                            })),
                    )
                })
                .when(!app.snapshot.modes.is_empty(), |picker| {
                    picker.child(
                        div()
                            .pt(px(7.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(picker_label("Mode"))
                            .children(app.snapshot.modes.iter().enumerate().map(
                                |(index, mode)| {
                                    let target = mode.id.clone();
                                    let selected =
                                        app.snapshot.selected_mode.as_deref() == Some(&mode.id);
                                    let entity = entity.clone();
                                    runtime_option(
                                        ("runtime-mode", index),
                                        mode.name.clone(),
                                        selected,
                                        move |cx| {
                                            let _ = entity.update(cx, |this, cx| {
                                                this.set_agent_mode(target.clone(), cx);
                                            });
                                        },
                                    )
                                },
                            )),
                    )
                }),
        )
        .with_priority(100)
        .into_any_element(),
    )
}

fn model_matches_query(name: &str, id: &str, query: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || id.to_ascii_lowercase().contains(query)
}

fn selected_provider<'a>(
    identity_provider: Option<&'a str>,
    catalog_provider: Option<&'a str>,
) -> &'a str {
    identity_provider.or(catalog_provider).unwrap_or_default()
}

fn picker_feedback(message: String) -> AnyElement {
    div()
        .p(px(8.0))
        .text_size(THEME.type_scale.caption)
        .text_color(THEME.colors.subtle)
        .child(message)
        .into_any_element()
}

fn picker_label(label: &'static str) -> AnyElement {
    div()
        .px(px(6.0))
        .py(px(3.0))
        .text_size(px(9.0))
        .text_color(THEME.colors.subtle)
        .child(label)
        .into_any_element()
}

fn runtime_option(
    id: impl Into<ElementId>,
    label: String,
    selected: bool,
    on_press: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let on_press = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .aria_selected(selected)
        .tab_index(0)
        .min_h(px(28.0))
        .px(px(7.0))
        .rounded(px(3.0))
        .flex()
        .items_center()
        .cursor(CursorStyle::PointingHand)
        .text_size(THEME.type_scale.caption)
        .text_color(if selected {
            THEME.colors.text
        } else {
            THEME.colors.muted
        })
        .when(selected, |option| option.bg(THEME.colors.hover))
        .hover(|option| option.bg(THEME.colors.hover))
        .on_click(move |_, _, cx| click(cx))
        .on_key_down(move |event, _, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(cx);
            }
        })
        .child(label)
        .into_any_element()
}

fn runtime_slash() -> AnyElement {
    div()
        .px(px(6.0))
        .text_color(THEME.colors.subtle)
        .child("/")
        .into_any_element()
}

fn effort_color(level: &str) -> gpui::Rgba {
    match level.to_ascii_lowercase().as_str() {
        "off" => THEME.colors.subtle,
        "minimal" => THEME.colors.muted,
        "low" => THEME.colors.link,
        "medium" => THEME.colors.accent,
        "high" => THEME.colors.warning,
        "xhigh" | "max" => THEME.colors.error,
        _ => THEME.colors.accent,
    }
}

fn effort_label(level: &str) -> String {
    let mut characters = level.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Off".into(),
    }
}

fn access_selector(
    selected: HarnessAccessMode,
    harness: &str,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let supported = crate::agents::supported_access_modes(harness);
    let selected = crate::agents::normalize_access_mode(harness, selected);
    let next = next_access_mode(selected, supported);
    let label = cycle_label(
        "Access",
        access_mode_label(selected),
        access_mode_label(next),
    );

    div()
        .id("harness-access")
        .flex_none()
        .child(access_cycle_button(
            "cycle-harness-access",
            AppIcon::Folder,
            access_mode_color(selected),
            label,
            move |cx| {
                let _ = entity.update(cx, |this, cx| this.set_access_mode(next, cx));
            },
        ))
        .into_any_element()
}

fn access_cycle_button(
    id: impl Into<ElementId>,
    resource: AppIcon,
    color: gpui::Rgba,
    label: String,
    on_press: impl Fn(&mut App) + 'static,
) -> AnyElement {
    let on_press = Rc::new(on_press);
    let click = Rc::clone(&on_press);
    let tooltip = label.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .flex()
        .items_center()
        .gap(px(5.0))
        .px(px(1.0))
        .py(px(2.0))
        .rounded(px(3.0))
        .cursor(CursorStyle::PointingHand)
        .text_color(THEME.colors.muted)
        .hover(|button| {
            button
                .bg(THEME.colors.surface)
                .text_color(THEME.colors.text)
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            click(cx);
        })
        .on_key_down(move |event, _, cx| {
            if activates_button(event) {
                cx.stop_propagation();
                on_press(cx);
            }
        })
        .child(app_icon(resource, AppIconSize::Inline))
        .child(div().size(px(7.0)).rounded_full().bg(color))
        .into_any_element()
}

fn cycle_label(capability: &str, current: &str, next: &str) -> String {
    format!("{capability}: {current}. Click to change to {next}")
}

fn next_access_mode(
    selected: HarnessAccessMode,
    supported: &[HarnessAccessMode],
) -> HarnessAccessMode {
    let Some(index) = supported.iter().position(|mode| *mode == selected) else {
        return supported
            .first()
            .copied()
            .unwrap_or(HarnessAccessMode::Full);
    };
    supported
        .get((index + 1) % supported.len().max(1))
        .copied()
        .unwrap_or(HarnessAccessMode::Full)
}

const fn access_mode_label(mode: HarnessAccessMode) -> &'static str {
    match mode {
        HarnessAccessMode::Full => "Full access",
        HarnessAccessMode::Sandboxed => "Sandboxed",
        HarnessAccessMode::Auto => "Auto",
    }
}

fn access_mode_color(mode: HarnessAccessMode) -> gpui::Rgba {
    match mode {
        HarnessAccessMode::Sandboxed => THEME.colors.warning,
        HarnessAccessMode::Auto => THEME.colors.accent,
        HarnessAccessMode::Full => THEME.colors.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_model_search_matches_names_and_ids() {
        assert!(model_matches_query("GPT-5 Codex", "gpt-5-codex", "codex"));
        assert!(model_matches_query("GPT-5 Codex", "gpt-5-codex", "gpt-5"));
        assert!(!model_matches_query("GPT-5 Codex", "gpt-5-codex", "claude"));
    }

    #[test]
    fn runtime_picker_uses_catalog_provider_before_session_start() {
        assert_eq!(selected_provider(None, Some("openai")), "openai");
        assert_eq!(
            selected_provider(Some("session-provider"), Some("catalog-provider")),
            "session-provider"
        );
    }

    #[test]
    fn access_controls_cycle_only_supported_modes() {
        use HarnessAccessMode::{Auto, Full, Sandboxed};
        assert_eq!(next_access_mode(Sandboxed, &[Sandboxed, Full]), Full);
        assert_eq!(next_access_mode(Full, &[Sandboxed, Full]), Sandboxed);
        assert_eq!(next_access_mode(Sandboxed, &[Sandboxed, Auto, Full]), Auto);
        assert_eq!(next_access_mode(Sandboxed, &[Auto, Full]), Auto);
    }
}
